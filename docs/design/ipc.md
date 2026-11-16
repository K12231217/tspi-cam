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

## 2. 精确报文示例（同时是测试夹具）

以下 JSON 块会被 `cargo xtask ipc` 从本文件抽出，与 `action-ipc/tests/wire.rs` 的夹具逐字节比对——
**文档里的例子就是测试用例**，改文档不改测试会红。

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
{"jsonrpc":"2.0","id":7,"result":{"frame_id":12034,"pts_ns":88123456789,"width":480,"height":800,"format":"nv12","stride":480,"payload_len":576000}}
```

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
`"params":null`）；`RecordGet`/`RecordDelete` 的参数是裸值（`"params":"20261116-141230-3f2a"`），
不做无意义的 `{id:…}` 包裹。

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
| `record.subscribe` | — | 无 result；之后持续收到 `record.progress`/`record.event` | `Stream` |
| `device.info` | — | `{sensor:{w,h,fps,mode:String}, encoders:{h264_fps_1080p,…}, imu:{who_am_i,odr_hz}, build}` | `Prompt` |
| `device.config.get` | — | 合并后有效配置（基础层 + 用户层） | `Prompt` |
| `device.config.set` | `{path:String, value:toml::Value}`（只写用户层） | 新有效配置 | `Operation` |
| `device.calibrate.enter` | — | `{lock:"held"}`（`actiond` 释放硬件，锁交给 `actioncal`） | `Operation` |
| `device.calibrate.leave` | — | `{reopened:true}` | `Operation` |
| `system.log.tail` | `{lines:u16}` | `{lines:[String]}` | `Slow` |
| `system.shutdown` | `{confirm:bool}` | `{ok:true}`（先停录） | `Operation` |

参数类型都定义在 `action-ipc::params`，每个都 `deny_unknown_fields`。

### 3.3 `Event`（通知）

```rust
#[serde(tag = "method", content = "params")]
pub enum Event {
    #[serde(rename = "record.progress")] RecordProgress(Progress),
    #[serde(rename = "record.event")]    RecordEvent(RecordEvent),
}
```

`record.event.class` 取值与 §6 错误码 `class` 一致。事件不是 `Call`，客户端不能把它当请求回复。

### 3.4 `media.frame` 不是 `Call`

只在 `/run/tspi-cam/media.sock` 上：先一个 JSON-RPC 响应头（见 §2），紧接着 `payload_len` 字节的
NV12 平面数据（luma 后 chroma，按 `stride` 排布，见 [interfaces.md](interfaces.md) §3）。
它**永不进入控制通道**，也永远不参与 `Call`/`Event` 路由——一帧 1080p 有 3.1 MB，混进控制面会把
`record.status` 的延迟从微秒级拖到毫秒级。

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

| socket | 模式 | 组 | 谁连 |
|---|---|---|---|
| `/run/tspi-cam/record.sock` | 0660 | `video` | `uid`、`actionctl` |
| `/run/tspi-cam/media.sock` | 0660 | `video` | `uid`（预览） |

- `actiond` 启动时建目录（systemd `RuntimeDirectory=tspi-cam`）→ bind → `fchown(0, video_gid)` →
  `chmod 0660` → 才开始服务。**组权限写死在 daemon 里，不靠 umask。**
- 客户端连不上必须是 `E_IPC` + 路径 + `strerror`，不允许静默重试到超时。

## 8. 门禁（`cargo xtask ipc`）

1. **协议示例即测试**：本文所有 ```json 块被抽成夹具，`deserialize→serialize` 必须逐字节等价；
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
