# tspi-cam IPC 契约

- 状态：v2（2026-11-16，按 review 修订）· 拥有者：`runtime/action-ipc`（唯一实现这份契约的 crate）
- 形态照抄 microduck 的 `duck-ipc-proto`：**JSON-RPC 2.0，一行一个对象（NDJSON），每服务一个 Unix socket**。
- 该 crate 只依赖 `serde` / `serde_json` / `semver`：所有服务（包括坏掉时还要能用的 `actionctl`）
  都编译它，不允许引入 tokio、http、crypto、图像库。

## 1. 报文类型（`action-ipc::wire`）

```rust
#[derive(Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: String,               // 恒为 "2.0"
    pub id: Id,                        // number 或 string；通知没有 id
    #[serde(flatten)]
    pub call: Call,                    // 见 §3，tag="method"、content="params"
}

#[derive(Serialize, Deserialize)]
pub struct Notification {              // 服务 -> 客户端，无 id
    pub jsonrpc: String,
    #[serde(flatten)]
    pub event: Event,                  // tag="method"、content="params"
}

#[derive(Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: String,
    pub id: Id,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}
```

- 分帧：单个 `\n`。请求可并发，靠 `id` 匹配。
- 进度/事件用**通知**（无 `id`）推送；客户端断线重连后重新 `record.subscribe` 即可继续收。
- 所有 `params` 结构体 `#[serde(deny_unknown_fields)]`：多一个字段就是 `INVALID_PARAMS` 并指名字段。
- **可选字段统一"省略"语义**：`?` 字段一律
  `#[serde(default, skip_serializing_if = "Option::is_none")]`，线上**不出现**该键；契约里不存在
  `"x":null`。要表达"清空某值"就用专门的方法，不用 null。
- **`result` 在传输层是 `serde_json::Value`，在契约层是类型**：每个方法的结果结构体定义在
  `action-ipc::results`（`Status`、`Started`、`IndexSummary`、`FrameHeader`、`ConfigView`…），
  server 必须序列化对应的结构体，客户端反序列化成同一个结构体。
  夹具校验用**语义相等**（解析成 `Value` 后比较），不是字节相等——`Value` 的 object 会按键排序，
  字节比较会惩罚一个正确的实现；这正是 review 抓到的 `media.frame` 假失败。
- `device.config.set` 的 `value` **不是 `toml::Value`**：TOML 的 datetime 会以
  `$__toml_private_datetime` 私有键泄漏进 JSON，且 JSON `null` 无法解码。用受限的
  `ConfigValue`（`Bool | Integer | Float | String`），数组/表暂不支持（要改就加专用方法）。

## 2. 精确报文示例（同时是测试夹具）

以下 JSON 块会被 `cargo xtask ipc` 从本文件抽出，与 `action-ipc/tests/wire.rs` 的夹具做**语义比对**
（解析成 `Value` 后相等；键序不比，见 §8.1）——**文档里的例子就是测试用例**，改文档不改测试会红。

请求：

```json
{"jsonrpc":"2.0","id":1,"method":"hello","params":{"api_version":1,"client":"actionctl/0.1.0"}}
```

```json
{"jsonrpc":"2.0","id":2,"method":"record.start","params":{"encode":{"width":1920,"height":1080},"codec":"h264","bitrate_kbps":10000}}
```

```json
{"jsonrpc":"2.0","id":3,"method":"record.stop","params":{"reason":"operator"}}
```

```json
{"jsonrpc":"2.0","id":4,"method":"record.delete","params":{"rec_id":"20261116-141230-3f2a","confirm":true}}
```

```json
{"jsonrpc":"2.0","id":5,"method":"record.subscribe","params":null}
```

订阅与预览的 ack（ack 之后才开始推送通知）：

```json
{"jsonrpc":"2.0","id":5,"result":{"subscribed":true}}
```

```json
{"jsonrpc":"2.0","id":6,"method":"media.subscribe","params":{"max_fps":10}}
```

```json
{"jsonrpc":"2.0","id":6,"result":{"subscribed":true,"width":480,"height":800,"format":"nv12"}}
```

通知（服务 → 客户端，无 `id`）：

```json
{"jsonrpc":"2.0","method":"record.progress","params":{"rec_id":"20261116-141230-3f2a","elapsed_ms":15320,"frames":459,"imu_samples":3080,"dropped":0}}
```

```json
{"jsonrpc":"2.0","method":"record.event","params":{"rec_id":"20261116-141230-3f2a","class":"E_STORAGE","code":1003,"message":"headroom below 500 MB, stopping"}}
```

成功响应：

```json
{"jsonrpc":"2.0","id":2,"result":{"rec_id":"20261116-141230-3f2a","state":"recording"}}
```

错误响应（`result` 不出现——JSON-RPC 2.0 §5：result 与 error 互斥；`Response::result` 为 `None` 时
`skip_serializing_if` 不输出）：

```json
{"jsonrpc":"2.0","id":2,"error":{"code":1001,"message":"already recording","data":{"class":"E_STATE"}}}
```

`media.frame` 的头部（二进制负载紧随其后，长度由 `payload_len` 给出）：

```json
{"jsonrpc":"2.0","method":"media.frame","params":{"frame_id":12034,"pts_ns":88123456789,"width":480,"height":800,"format":"nv12","stride":480,"payload_len":576000}}
```
（这一行之后紧跟恰好 `payload_len` 字节的 NV12 数据，见 §3.4。）

## 3. `Call`：方法名与参数在类型上成对绑定

**每个变体显式指定线上名**（`#[serde(rename = "...")]`），不依赖 Rust 变体名的大小写规则。
线上名是**永久契约**：改名 = 破坏性变更，必须 `API_VERSION + 1` 并在 `CHANGELOG` 的 `ipc:` 段写明。

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum Call {
    #[serde(rename = "hello")]                    Hello(HelloParams),
    #[serde(rename = "record.status")]            RecordStatus,
    #[serde(rename = "record.start")]             RecordStart(StartParams),
    #[serde(rename = "record.stop")]              RecordStop(StopParams),
    #[serde(rename = "record.list")]              RecordList(ListParams),
    #[serde(rename = "record.get")]               RecordGet(RecId),
    #[serde(rename = "record.delete")]            RecordDelete(DeleteParams),
    #[serde(rename = "record.subscribe")]         RecordSubscribe,
    #[serde(rename = "media.subscribe")]          MediaSubscribe(MediaSubscribeParams),
    #[serde(rename = "media.unsubscribe")]        MediaUnsubscribe,
    #[serde(rename = "device.info")]              DeviceInfo,
    #[serde(rename = "device.config.get")]        DeviceConfigGet,
    #[serde(rename = "device.config.set")]        DeviceConfigSet(ConfigPatch),
    #[serde(rename = "device.calibrate.enter")]   DeviceCalibrateEnter,
    #[serde(rename = "device.calibrate.leave")]   DeviceCalibrateLeave,
    #[serde(rename = "system.log.tail")]          SystemLogTail(LogParams),
    #[serde(rename = "system.shutdown")]          SystemShutdown(ShutdownParams),
}
```

单元变体（`RecordStatus`、`DeviceInfo`、`RecordSubscribe`）**不带 `params` 字段**，线上就是
`{"jsonrpc":"2.0","id":4,"method":"record.status"}`（实测：adjacently-tagged 枚举的 unit 变体不输出
`"params":null`）；只有 `RecordGet` 的参数是裸值（`"params":"20261116-141230-3f2a"`，单个标识符无需
包裹），`RecordDelete` 固定为对象 `{rec_id,confirm}`——删除是不可撤销操作，参数拒绝裸值。

### 3.1 解码必须经过两步（否则错误码无法区分）

**不要直接给 `Request` derive `Deserialize`。** 实测：adjacently-tagged 枚举会把未知方法变成
`unknown variant \`record.nope\`, expected one of …` 的**解码错误**，这样 `-32601` 与 `-32602` 就分不开，
也无法在 `data` 里报出方法名/字段名。

`action-ipc` 必须提供：

```rust
impl Request {
    /// 1) 先把 `method` 当字符串取出；未知方法 → METHOD_NOT_FOUND { method }
    /// 2) 再按该变体的 params 类型解码；失败 → INVALID_PARAMS { field }
    pub fn decode(line: &str) -> Result<Self, RpcError>;
}
```

实现上先用 `#[derive(Deserialize)] struct Envelope { jsonrpc, id, method: String, params: Option<Value> }`
读外壳，再 `match method.as_str()` 到 `Call` 变体并逐个 `serde_json::from_value::<Params>(raw)`。
**`Call` 的 `Deserialize` 只用于测试与文档夹具，线上路径一律走 `decode`**——这条差异必须写在
`action-ipc` 的模块注释里，否则实现阶段一定有人直接 `from_str` 然后丢掉错误码。

### 3.2 方法与参数/结果

| 线上名 | params | result | lane |
|---|---|---|---|
| `hello` | `{api_version:u32, client:String}` | `{api_version, server, build, pid}` | `Prompt` |
| `record.status` | — | `{state, rec_id?, elapsed_ms, frames, imu_samples, dropped, disk_free_mb, camera:{width,height,fps}, encode:{codec,bitrate_kbps}, last_error?}` | `Prompt` |
| `record.start` | `{encode:{width,height}, codec:"h264"\|"h265", bitrate_kbps:u32, note?:String}` | `{rec_id, state}` | `Operation` |
| `record.stop` | `{reason:String}` | `IndexSummary`（见 record-format.md） | `Operation` |
| `record.list` | `{limit:u16, before?:RecId}` | `{records:[IndexSummary], next?:RecId}` | `Slow` |
| `record.get` | `RecId` | `IndexSummary` | `Slow` |
| `record.delete` | `{rec_id:RecId, confirm:bool}` | `{deleted_bytes:u64}` | `Operation` |
| `record.subscribe` | `null` | `{subscribed:true}`（ack 之后才开始推送通知；连接关闭即退订） | `Stream` |
| `device.info` | — | `{sensor:{w,h,fps,mode:String}, encoders:{h264_fps_1080p,…}, imu:{who_am_i,odr_hz}, build}` | `Prompt` |
| `device.config.get` | — | 合并后有效配置（基础层 + 用户层） | `Prompt` |
| `device.config.set` | `{path:String, value:ConfigValue}`（只写用户层） | `ConfigView`（合并后有效配置） | `Operation` |
| `device.calibrate.enter` | — | `{lock:"held"}`（`actiond` 释放硬件，锁交给 `actioncal`） | `Operation` |
| `device.calibrate.leave` | — | `{reopened:true}` | `Operation` |
| `system.log.tail` | `{lines:u16}` | `{lines:[String]}` | `Slow` |
| `system.shutdown` | `{confirm:bool}` | `{ok:true}`（先停录） | `Operation` |
| `media.subscribe` | `{max_fps:u8}` | `{subscribed:true,width,height,format}`（仅 `media.sock`） | `Stream` |
| `media.unsubscribe` | `null` | `{subscribed:false}`（之后不再推帧，连接可复用） | `Prompt` |

参数类型都定义在 `action-ipc::params`，每个都 `deny_unknown_fields`。

```rust
/// 配置值只允许标量；datetime/数组/表一律不支持（见 §1）。
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub enum ConfigValue { Bool(bool), Integer(i64), Float(f64), String(String) }
```

### 3.3 `Event`（通知）

```rust
#[serde(tag = "method", content = "params")]
pub enum Event {
    #[serde(rename = "record.progress")] RecordProgress(Progress),
    #[serde(rename = "record.event")]    RecordEvent(RecordEvent),
    #[serde(rename = "media.frame")]     MediaFrame(results::FrameHeader), // 后跟二进制负载
}
```

- `record.event.class` 取值与 §6 错误码 `class` 一致。
- `media.frame` 只出现在 `media.sock` 上，且只在 `media.subscribe` 的 ack 之后发送。
- 事件不是 `Call`，客户端不能把它当请求回复；收到不认识的通知必须**忽略**（不报错、不断连），
  这样新增事件不会让旧客户端失效。

### 3.4 `media.sock` 上的帧协议

`media.frame` 不是 `Call`：它的线上形态是**一行 JSON-RPC 通知 + 恰好 `payload_len` 字节负载**，
接在同一个连接上，可重复。

```text
{"jsonrpc":"2.0","method":"media.frame","params":{...,"payload_len":576000}}

<576000 bytes NV12>
{"jsonrpc":"2.0","method":"media.frame","params":{...}}

<bytes>
```

- 分帧规则（唯一）：读到 `\n` 之前是头；头里 `payload_len` 决定后面要读的字节数；读完立刻进入下一条头。
  **负载里可以出现 `\n`，不允许按行扫描负载。**
- 客户端先 `media.subscribe {max_fps}`，收到 `{subscribed:true,width,height,format}` 后才开始收帧；
  `media.unsubscribe` 或关闭连接即停止（不需要 1:1 的退订确认）。
- 服务端对慢客户端**丢帧不阻塞**：发送队列上限 2 帧，满则丢最旧帧并计入 `stats.preview_dropped`；
  永不反压采集/编码。
- 负载格式 NV12，按 `stride` 排布（[interfaces.md](interfaces.md) §3.2）。一帧 1080p 有 3.1 MB，
  所以它**永不进入控制通道**，也永远不参与 `Call` 路由。

### 3.5 录制状态取值（契约，不是实现细节）

`record.status.state` 与 `record.start/stop` 的返回值只允许三个值：

| state | 含义 | 允许的调用 | 拒绝 |
|---|---|---|---|
| `idle` | 未录制，硬件已释放 | `record.start`、`device.calibrate.enter`、`system.shutdown` | `record.stop` → `E_STATE{reason:"not_recording"}` |
| `recording` | 正在录，硬件由 `actiond` 持有 | `record.stop`、`record.status`、`record.subscribe` | 第二次 `record.start` → `E_STATE{reason:"already_recording"}`；`device.calibrate.enter` → `E_STATE{reason:"recording"}` |
| `stopping` | Drain/Flush/Finalize 中 | `record.status`（返回 `stopping` + 进度）、`record.subscribe` | `record.start`/`record.stop` → `E_STATE{reason:"busy_stopping"}` |

- `E_STATE` 的 `data.reason` 取值固定为 `{already_recording, not_recording, busy_stopping, recording, fault}`，
  客户端按 `reason` 分支，**不解析 `message` 文本**。
- `fault` 不在 `state` 里：故障表现为"记录未完成"（`index.json.complete=false`）+ `record.event`
  推送 `E_*`，daemon 仍可 `record.start`（重启一次会话）。状态机细节归 `record-format.md`。

## 4. 版本规则（microduck 的结论，照抄）

```rust
pub const API_VERSION: u32 = 1;   // 任何不兼容改动 +1
```

- **版本不同不拒绝任何调用**，只在 `hello` 的 result 里回报。真正失败的只有两种情况：
  方法不存在（`METHOD_NOT_FOUND`，报方法名）或参数形状变了（`INVALID_PARAMS`，报字段名）。
- 理由：在握手上拒绝会让本可服务的调用一起失败；所有二进制总是一起发布，安装器保证。

## 5. lane 与超时

| lane | 语义 | 客户端超时 | 服务端并发 |
|---|---|---|---|
| `Prompt` | 纯内存查表 | 500 ms | 可并发 |
| `Slow` | 读盘/扫目录 | 5 s | 可并发 |
| `Operation` | 改变状态，可能几十秒 | 120 s | 同一连接串行；跨连接由状态机裁决 |
| `Stream` | 不返回 result，写通知直到断开 | — | 每连接一个 |

## 6. 错误码

| 数字 | class | 含义 |
|---|---|---|
| -32700 | `PARSE` | 不是合法 JSON |
| -32601 | `METHOD_NOT_FOUND` | 方法不存在（data 带方法名） |
| -32602 | `INVALID_PARAMS` | 参数不合法（data 带字段名） |
| 1001 | `E_STATE` | 非法状态转移（`ALREADY_RECORDING`/`NOT_RECORDING`/`BUSY_STOPPING`） |
| 1002 | `E_HW` | 设备打不开/丢失/编码器失败 |
| 1003 | `E_STORAGE` | 余量不足、写盘失败 |
| 1004 | `E_CFG` | 配置不合法（分辨率不在白名单等） |
| 1005 | `E_IPC` | socket/权限/对端断开 |
| 1006 | `E_CALIB` | 标定文件缺失/损坏/版本不符 |
| 1007 | `E_ENC_LIFECYCLE` | 编码器输入/输出不配对（见 interfaces.md §4） |

错误码随 `record.event` 与 `index.json.errors` 落盘，不允许只打日志。

## 7. Socket 与权限

| socket | 模式 | 属主 | 谁连 |
|---|---|---|---|
| `/run/tspi-cam/record.sock` | 0660 | `root:video` | `uid`、`actionctl`、`actioncal` |
| `/run/tspi-cam/media.sock` | 0660 | `root:video` | `uid`（预览） |

- **socket 由 systemd 创建**（`actiond.socket`：`SocketMode=0660`、`SocketUser=root`、
  `SocketGroup=video`），`actiond` 以 `tspi-cam` 身份 `accept` 传入的 fd。daemon **不自建 socket、
  不 fchown**——非 root 进程无法把属主改成 `root:video`（EPERM），这正是必须用 socket activation 的原因。
- `connect()` 需要写权限（unix(7)），所以 `0640` 连不上，`0660` 是下限。
- 客户端连不上必须是 `E_IPC` + 路径 + `strerror`，不允许静默重试到超时。

## 8. 门禁（`cargo xtask ipc`）

1. **协议示例即测试**：本文所有 ```json 块被抽成夹具。请求方向要求"用类型构造再序列化"的
   `Value` 与夹具**语义相等**；响应方向要求"用 `results::*` 类型构造"的 `Value` 与夹具语义相等。
   不比较字节（键序由 `serde_json::Map` 决定），但**集合与类型必须完全一致**；
2. `Call::method()` 与 `#[serde(rename)]` 一一对应，无重复、无孤儿；
3. 每个 `Call` 变体都有 lane，且与 §3.2 表一致；
4. 每个 `params` 结构体都有 `deny_unknown_fields`；
5. `Request::decode`：未知方法 → `METHOD_NOT_FOUND`（data 带方法名）、未知字段 → `INVALID_PARAMS`
   （data 带字段名），各一个测试；
6. 单元变体（`record.status`）与裸值参数（`record.get`）各一个往返夹具，断言线上**没有**
   `params` 字段 / `params` 是标量；
7. 错误响应序列化结果中不出现 `result`；
8. `API_VERSION` 变更必须同时改 `CHANGELOG` 的 `ipc:` 段。

每条都要有一次"故意让它红"的记录（负向验证），否则不算门禁。
