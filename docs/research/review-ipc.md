# IPC 契约对抗性 review（tspi-cam）

- 审阅对象：`docs/design/ipc.md`、`docs/design/interfaces.md`（并交叉核对 `docs/design/architecture.md`、`docs/design/README.md`）
- 快照：`HEAD = 2fe1ef2`（2026-11-16 19:53），工作区另有未提交修改（`ipc.md` 20:01、`interfaces.md` 19:59、`architecture.md` 20:00）。**下面的行号以该工作区快照为准**；若后续继续改动，请按引用的原文 grep 定位。
- 方法：WSL Ubuntu / cargo 1.100.0 / rustc 1.100.0；serde 1 + serde_json 1 走本机 registry 缓存（`--offline` 可用，无需网络）；另用 toml 0.8 做了 wire 探测。探针工程在 `~/ipctest`、`~/tomltest`，未写入仓库。

---

## 0. 先回答三个指定问题（探针实测）

### 0.1 adjacently-tagged + flatten 的真实线上形态

用与文档相同的定义（`#[serde(tag="method", content="params")]` + `Request { jsonrpc, id, #[serde(flatten)] call }`）实测：

| 输入/类型 | 输出 | 结论 |
|---|---|---|
| `Call::RecordStatus` 单独序列化 | `{"method":"record.status"}` | unit 变体**不输出** `params` |
| `Request{id:2, RecordStatus}` | `{"jsonrpc":"2.0","id":2,"method":"record.status"}` | flatten 后 `method` 紧跟 `id`，无 `params` |
| 反序列化 `{"method":"record.status","params":null}` | 成功，再序列化后 `params` 消失 | `params:null` 被**接受但丢弃** |
| 反序列化 `{"method":"record.subscribe","params":{"bogus":1}}` | `invalid type: map, expected unit variant ...` | 直接 derive 会拒绝对象/数组 |
| `record.get` + `"params":"2026..."` | round-trip 逐字节相等 | 裸值参数成立 |
| 未知方法 | `unknown variant \`record.nope\`, expected one of ...` | 与 §3.1 的描述一致 |
| 顶层多一个字段 | **被静默接受**（flatten 下无法 deny） | 需要在 Envelope 层处理 |

结论：**当前 §3 的“unit 变体不带 params”说法是对的**。adca9f8 里“序列化为 `"params":null`”是错的，但已被 2fe1ef2 修掉；本 review 不再把它算缺陷，只提醒实现时 Envelope 路径必须显式拒绝 unit 变体的非 null `params`（否则“多一个字段就是 INVALID_PARAMS”在 unit 方法上不成立）。

### 0.2 错误响应里的 `result:null`

adca9f8 的示例与 `skip_serializing_if` **确实矛盾**：`Option<Value>` 遇到 JSON `null` 反序列化为 `None`，再序列化时整个 `result` 被跳过。2fe1ef2 已把示例改为：

```json
{"jsonrpc":"2.0","id":2,"error":{"code":1001,"message":"already recording","data":{"class":"E_STATE"}}}
```

探针确认这条现在 `deserialize → serialize` **逐字节相等**。

### 0.3 当前 §2 仍有两条 fixture 过不了 §8.1

用文档自己的类型跑 `deserialize → serialize`：

| fixture | 结果 | 原因 |
|---|---|---|
| `hello`、`record.stop`、`record.progress`、`record.event`、错误响应、`record.get` | byte-equal ✅ | — |
| **`record.start`（ipc.md:53）** | **byte-equal ❌** | `note: Option<String>` 没有 skip 属性，输出多出 `,"note":null` |
| **`media.frame` 头（ipc.md:86）** | **byte-equal ❌** | `result: Option<serde_json::Value>` 默认 `BTreeMap`，键被字典序重排 |

实测输出：

```text
start:  ...{"encode":{...},"codec":"h264","bitrate_kbps":10000,"note":null}
frame:  {"jsonrpc":"2.0","id":7,"result":{"format":"nv12","frame_id":12034,"height":800,"payload_len":576000,"pts_ns":88123456789,"stride":480,"width":480}}
```

这两条是下文的 B1、B2；不是风格问题，而是 §8 门禁在当前文档上**必然红**。

### 0.4 toml::Value 放到 JSON wire 上的实测

```text
toml_to_json = {"a":1,"b":2.5,"c":true,"d":"x","e":{"$__toml_private_datetime":"1979-05-27T07:32:00Z"},"t":[1,2]}
json_back_to_toml = OK（私有键能还原）
null -> ERR invalid type: null, expected any valid TOML value
```

TOML 的 datetime 会以 `$__toml_private_datetime` 这种 TOML 私有结构泄漏到 JSON 契约里，而且 JSON `null` 无法解码。见 B6。

### 0.5 已被工作区修掉的旧问题（不要重复报）

- `fchown(0, video_gid)` 与非 root `actiond` 的矛盾：工作区已改为 systemd socket activation（`ipc.md:226-228`，`architecture.md` §3 明确写了 EPERM）。这条现在两页一致。
- unit 变体 `params:null`、错误响应 `result:null`：2fe1ef2 已修，探针复核通过。

---

## 1. Blocking（必须在冻结前改）

### B1. `record.start` fixture 与 `note?:String` 自相矛盾，§8.1 必红（P1，置信度 0.98）

- **位置**：`docs/design/ipc.md:53`（fixture）、`:148`（`note?:String`）、`:234`（门禁 1）
- **错在哪**：文档只规定了 `deny_unknown_fields`，没有规定可选字段的序列化规则。`Option<String>` 默认会把 `None` 写成 `"note":null`，而 fixture 省略了 `note`。于是“文档示例 = 测试夹具”在第一条 `record.start` 上就失败。`record.status.rec_id?`、`last_error?`、`record.list.before?`、`next?` 也有同样问题，只是暂时没有 fixture。
- **建议改法**：在 §1 增加一条硬规则，并把所有可选字段落到类型上：

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub note: Option<String>,
```

  同时在 §3.2 表下写明：“`?` 字段默认省略；需要显式 null 的字段必须单独标注并加 fixture”。如果团队更想要 `null` 语义，就反过来把 `"note":null` 写进 fixture 并禁止 skip，但**不能两种混用**。
- **证明测试**：`action-ipc/tests/wire.rs` 里 `record_start_fixture_roundtrip`，输入就是文档第 53 行的字节；按当前类型运行会得到 `note:null`（上表实测），加 skip 后通过（探针已验证 `eq=true`）。

### B2. `media.frame` fixture 与 `result: Option<Value>` 矛盾；且响应结果完全没有类型（P1，置信度 0.98）

- **位置**：`docs/design/ipc.md:31`（`result: Option<serde_json::Value>`）、`:86`（fixture）、`:234`
- **错在哪**：`serde_json::Value` 的 object 默认按键排序，反序列化再序列化会把 `frame_id, pts_ns, ...` 重排成 `format, frame_id, height, payload_len, pts_ns, stride, width`，§8.1 必红（实测见 0.3）。更深一层的问题：所有 `result` 都是无类型 `Value`，`Call` 有类型、`result` 没有，两端只能靠运行时约定；`record.start` 的 fixture 只是碰巧键序是字典序，才通过了。
- **建议改法**（推荐）：给每个方法定义 result 类型，`Response` 用带类型的枚举/泛型，而不是 `Value`：

```rust
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameHeader {
    pub frame_id: u64,
    pub pts_ns: u64,
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
    pub stride: u32,
    pub payload_len: u32,
}
```

  若暂时不想改 `Response`，最小修法是：fixture 一律写成**字典序**，或给 `serde_json` 开 `preserve_order`（会引入 indexmap 传递依赖，和“只依赖 serde/serde_json/semver”的表述冲突，需要一并决定）。但只修 fixture 不解决客户端没有类型可匹配的问题。
- **证明测试**：`media_frame_fixture_roundtrip`（用第 86 行字节）。当前类型下输出顺序如 0.3 所示，断言失败；改成 `FrameHeader` 或 `preserve_order` 后通过。

### B3. `record.delete` 的参数形状在同一页里出现三种说法（P1，置信度 0.99）

- **位置**：`docs/design/ipc.md:118`（“`RecordGet`/`RecordDelete` 的参数是裸值”）、`:104`（`RecordDelete(DeleteParams)`）、`:152`（`{rec_id:RecId, confirm:bool}`）
- **错在哪**：prose 说 `record.delete` 是裸字符串，枚举和表都说它是带 `confirm` 的对象。`actionctl` 按 prose 实现、`uid` 按表实现，第一次删除就会 `INVALID_PARAMS`；更糟的是 fixture 里根本没有 `record.delete`，门禁抓不到。
- **建议改法**：把第 118 行改成只描述 `record.get`：

```text
`RecordGet` 的参数是裸值（`"params":"20261116-141230-3f2a"`），
不做无意义的 `{id:…}` 包裹；`record.delete` 需要 `confirm`，参数是 `{rec_id, confirm}` 对象。
```

  并在 §2 增加一条 `record.delete` 的请求/响应 fixture。
- **证明测试**：`record_delete_fixture_roundtrip`：用 `{"params":{"rec_id":"...","confirm":true}}` 反序列化并通过；同时 `xtask ipc` 增加一条检查：`RecordDelete` 的参数类型不能是 `RecId` 裸值（按方法名/类型文本或编译期类型断言）。

### B4. `record.subscribe` 声明“无 result”，客户端拿不到合法的 JSON-RPC 应答（P1，置信度 0.95）

- **位置**：`docs/design/ipc.md:153`（“无 result”）、`:200`（Stream“不返回 result”）、`:30-33`（`Response` 的 skip 规则）
- **错在哪**：如果服务端按 `Response { result: None, error: None }` 回一条 ack，序列化结果是 `{"jsonrpc":"2.0","id":4}`（探针实测）——既没有 `result` 也没有 `error`，不是合法 JSON-RPC 2.0 响应，客户端无法判断订阅是否成功，也无法把 `E_IPC`/`E_STATE` 错误带回来。如果服务端干脆不回，客户端同样无法区分“已订阅”和“请求被丢弃”。另外断线重连语义只写了一句，没有 ack 就无法确认重连成功。
- **建议改法**：给 subscribe 明确一个成功 result，并把它与后续通知分开：

```json
{"jsonrpc":"2.0","id":4,"result":{"subscribed":true}}
```

  表里 result 改成 `{subscribed:true}`；lane 表改成“先回 ack，之后持续推送通知，直到连接关闭或 `record.unsubscribe`”。如果没有 unsubscribe，就写明**关闭连接是唯一的退订方式**，并规定服务端在 ack 之后才允许推送第一条通知（避免客户端在 ack 前收到事件）。
- **证明测试**：`subscribe_ack_is_valid_jsonrpc`：发送 `record.subscribe`，断言收到一行响应，且 `result XOR error` 恰好一个非空；再加一个 `subscribe_before_notification` 的集成测试，确保 ack 先于第一条 `record.progress`。

### B5. `media.frame` 没有发起方式、没有生命周期、没有关闭语义（P1，置信度 0.95）

- **位置**：`docs/design/ipc.md:83-87`、`:176-181`
- **错在哪**：§3.2 的 15 个方法里没有任何 media 方法；§3.4 却说这是“JSON-RPC 响应头”，带着 `"id":7`。没有对应的请求，`id` 无从匹配；没有 subscribe/unsubscribe，客户端无法决定何时开始/停止收帧，服务端也无法知道该给谁推。关闭语义、慢客户端丢帧、`payload_len` 与实际二进制不一致、解析到一半断线，全都没有写。`uid` 和 `actiond` 会各自发明一套。
- **建议改法**：在 media socket 上定义一个独立的、极小的平面（不要塞进控制面 `Call`）：

```rust
#[serde(tag = "method", content = "params")]
pub enum MediaCall {
    #[serde(rename = "media.subscribe")]   Subscribe(MediaParams),
    #[serde(rename = "media.unsubscribe")] Unsubscribe,
}
pub struct MediaParams { width: u32, height: u32, fps: u8, format: PixelFormat }

// 帧是通知，不是响应；没有 id，避免“一个请求多个响应”的非标准语义
#[derive(Serialize, Deserialize)]
pub struct MediaFrame {
    pub jsonrpc: String,          // "2.0"
    pub method: String,           // "media.frame"
    pub params: FrameHeader,      // 见 B2
}
```

  并规定：
  1. 头部是**恰好一行** `\n` 结尾的 JSON，随后紧跟 `payload_len` 字节，期间没有其他字节；
  2. 帧可以因慢客户端被丢弃，`frame_id` 不保证连续；
  3. 关闭 socket 或 `media.unsubscribe` 即停止推送，服务端不保证发“结束帧”；
  4. `payload_len` 必须等于 `stride * height * 3 / 2`（与 interfaces §3.2 一致），否则视为协议错误并断开；
  5. 媒体 socket 上的错误（无相机、权限、E_HW）必须在 ack 里返回 `error`，不能静默断开。
- **证明测试**：`media_subscribe_ack_then_notifications`：连接 media.sock → 发送 `media.subscribe` → 断言 ack → 收到至少一帧通知 → 发送 `media.unsubscribe` → 断言不再有帧且 socket 仍可关闭；另加一个 `frame_fixture_framing`：把“JSON 行 + 576000 字节”作为一个整体喂给解析器，断言不越界、下一帧从正确偏移开始。

### B6. `device.config.set` 把 `toml::Value` 放进 wire，违反依赖规则且污染 JSON（P1，置信度 0.99）

- **位置**：`docs/design/ipc.md:5-6`（action-ipc 只依赖 serde/serde_json/semver）、`:156`（`value:toml::Value`）、`docs/design/interfaces.md:14`（同样的依赖表）
- **错在哪**：`action-ipc` 不能依赖 `toml`（依赖表明确禁止），而 `ConfigPatch` 定义在 `action-ipc::params`；`action-config` 又依赖 `action-ipc`，不能反过来给 ipc 提供类型。即便允许 toml，实测 datetime 会变成 `{"$__toml_private_datetime":...}`，JSON `null` 直接解码失败；这是把 TOML 的实现细节泄漏到线上协议。
- **建议改法**：wire 上用 JSON 原生类型，转换放在 `action-config`：

```rust
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigPatch {
    pub path: String,
    pub value: serde_json::Value,
}
```

  `action-config` 收到后 `toml::Value::try_from(value)`，失败（例如 `null`、JSON 对象/数组形状不合法）返回 `E_CFG`，并带上 `path`。同时在 §3.2 写明 `path` 的语法（点分键，如 `camera.fps`；是否允许数组索引）。
- **证明测试**：`config_patch_wire_is_json_native`：`serde_json::to_string(ConfigPatch { value: json!({"a":1,"e":"1979-05-27T07:32:00Z"}) })` 不出现 `$__toml_private_datetime`；`serde_json::from_str("null")` 能进入 `serde_json::Value`，由 action-config 返回 `E_CFG`，而不是在 ipc 层解析失败。再加 `cargo tree -p action-ipc | grep toml` 为空（`xtask arch`）。

### B7. `IndexSummary` 被引用但定义页 `record-format.md` 还没写（P1，置信度 0.95）

- **位置**：`docs/design/ipc.md:149-151`；`docs/design/README.md:11`（record-format.md 状态“待写”）
- **错在哪**：`record.stop`、`record.list`、`record.get` 的 result 全是 `IndexSummary`，`xtask smoke` 还要核对 `index.json`。这个类型没有字段表，就没有可实现的类型，也没有 fixture；§8.1 覆盖不到它，`record.stop` 作为 M1 的主路径会在实现阶段被临时发明。
- **建议改法**：冻结 IPC 前先落 `record-format.md`，至少给出 `IndexSummary` 的字段、类型和可选性（`rec_id, started_at, duration_ms, frames, frames_dropped, imu_samples, bytes, complete, error_count, last_error?`），并在 ipc.md 为 `record.stop`/`record.list`/`record.get` 各加一条 result fixture。若暂时不写该页，就把字段表内联进 ipc.md 并标注“record-format.md 出来后以该页为准”，不要留一个仅名字的引用。
- **证明测试**：`stop_result_fixture_roundtrip`、`list_result_fixture_roundtrip`、`get_result_fixture_roundtrip`；在 `IndexSummary` 类型不存在时测试无法编译，这就是门禁。

### B8. 状态机没有线上取值；“停止过程中查询/再次 start”没有定义（P1，置信度 0.92）

- **位置**：`docs/design/ipc.md:147`（`state` 无取值表）、`:209`（`E_STATE` 只列了 `ALREADY_RECORDING`/`NOT_RECORDING`/`BUSY_STOPPING`，没有对应到方法）、`:153`
- **错在哪**：`record.status` 的 `state` 是字符串，但文档从未定义合法值；fixture 里出现 `"recording"`，UI 只能猜 `"idle"/"stopping"` 是否存在。停止可能持续几十秒（Operation lane 120 s），这期间 `record.status` 应该返回什么、`record.start`/`record.stop` 应该返回哪个子原因、`rec_id` 是否还在，全都没有写。这正是实现阶段两端最容易对不上的地方（UI 显示空白或把 stopping 当成 idle）。
- **建议改法**：在 §3.2 增加状态表：

```text
state ∈ {"idle", "recording", "stopping"}
rec_id 仅当 state != "idle" 时出现（或明确为 null）
start: idle -> recording；recording -> E_STATE/ALREADY_RECORDING；stopping -> E_STATE/BUSY_STOPPING
stop:  recording -> stopping -> idle；idle -> E_STATE/NOT_RECORDING；stopping -> E_STATE/BUSY_STOPPING
status: 任何时候都可调用；stopping 时 elapsed/frames 冻结还是继续增长要明确
```

  并把 `E_STATE` 的 `data` 结构写成 `{"class":"E_STATE","reason":"ALREADY_RECORDING"}`（见 S3），让客户端不必解析英文 `message`。
- **证明测试**：状态机单测（host，fake sink）：`start → 卡住的 stop → status` 返回 `{"state":"stopping","rec_id":...}`；`status` 在 500 ms 内返回，不受 stop 影响；`start` 在 stopping 时返回 `E_STATE` 且 `data.reason == "BUSY_STOPPING"`。再补一个“同一连接上 stop 进行中调用 status”的集成测试（见 S10）。

---

## 2. Should fix

### S1. `record.start` 的分辨率/fps/编码器错误语义没有闭合（P2，置信度 0.9）

`ipc.md:147-148` 的 `record.start` 只有 `encode{width,height}`、`codec`、`bitrate_kbps`，没有 `fps`；interfaces.md:210/220 却要求 `encode.geom` 来自配置并且 `fps ≤ ENC_FPS_CEIL_*`。到底请求参数覆盖配置，还是配置说了算？`record.start` 的 `encode` 与 `device.config.set` 改的 `encode.geom` 谁优先？`record.status.encode` 又不返回 width/height，UI 无法确认当前输出几何。另外 `codec` 的 wire 值（`"h264"`/`"h265"`）、非法值的错误码（`INVALID_PARAMS` 还是 `E_CFG`）、白名单不通过（`E_CFG`）与硬件打开失败（`E_HW`）的优先级没有规定。

建议：明确“请求参数只对本次录制生效，不写配置；`fps` 从配置读，默认 30”；`record.status.encode` 补 `width/height/fps`；加一张错误映射表：语法/类型 → `INVALID_PARAMS`，语义/白名单/fps 上限 → `E_CFG`，设备/编码器打开失败 → `E_HW`。测试：`start_with_720p60` 返回 `E_CFG` 且 `data.field == "fps"`；`start_with_codec:"h266"` 返回 `INVALID_PARAMS` 或 `E_CFG`（二选一，写进文档）。

### S2. 磁盘满/编码器故障自动停止时，没有结构化的“已停止”事件和最终摘要（P2，置信度 0.9）

`ipc.md:67` 的 `record.event` 只有 `class/code/message`，`message` 是自由文本；`record.status` 也不保证能拿到最终 `IndexSummary`。录制因 `E_STORAGE`（或 `E_HW`/`E_ENC_LIFECYCLE`）自动停下后，订阅者只知道“发生了错误”，不知道已经停止、也不知道最终帧数和错误列表；`record.stop` 的 result 只回给调用 stop 的人，而自动停止没有调用者。

建议：增加 `record.stopped` 通知（`rec_id, reason, index: IndexSummary`），或规定自动停止后 `record.event` 的 `data` 必须包含 `stopped:true` 和一个可用 `record.get` 取的 `rec_id`；同时规定 `record.status` 在自动停止后返回 `state:"idle"` 且 `last_error` 保留。测试：模拟磁盘余量低于阈值，断言收到 `record.stopped`（或带 `stopped:true` 的 event），随后 `record.get(rec_id)` 返回 `complete:false/true` 的摘要。

### S3. 错误码表不完整，`data` 形状没有定义，错误子原因丢失（P2，置信度 0.95）

- 没有 `-32600 INVALID_REQUEST`：合法 JSON 但缺 `method`、`jsonrpc != "2.0"`、`id` 类型非法时，§6 没有对应码。探针显示 `jsonrpc:"1.0"` 会被接受并原样回显；`id:null` 会以“did not match any variant”失败。这两种情况现在只能落到 `PARSE` 或 `PARSE` 之外的临时码。
- `METHOD_NOT_FOUND` 的 `data` 写“带方法名”，`INVALID_PARAMS` 写“带字段名”，但没有给出 key。`E_STATE` 的 fixture 反而用 `data:{"class":"E_STATE"}`，把 `ALREADY_RECORDING`/`NOT_RECORDING`/`BUSY_STOPPING` 三个子原因丢掉了——客户端只能解析英文 `message`。
- 类型错误无法给出字段名：探针里 `{"reason":5}` 的错误是 `invalid type: integer \`5\`, expected a string`，没有 `reason`；嵌套错误只说 `bogus`，不说 `encode.bogus`。这与 §3.1 和 §8.5 的“报字段名”承诺不一致。

建议：补 `-32600`；给每种错误定义 `data` schema（至少 `{"method":...}`、`{"field":...}`、`{"class":"E_STATE","reason":"ALREADY_RECORDING"}`、`{"class":"E_CFG","path":"camera.fps"}`）；明确 `INVALID_PARAMS` 的 `data.field` 只在能确定时出现，嵌套路径用 `encode.bogus` 形式；要么增加 `serde_path_to_error`（会改依赖表），要么对顶层字段做显式校验。测试：`decode_unknown_field`、`decode_wrong_type_reason`、`decode_bad_jsonrpc`、`decode_id_null` 四个断言，检查 `data` 的 key 和值。

### S4. §8.1 的“文档 JSON 块 = 测试夹具”缺少可执行的抽取规则（P2，置信度 0.95）

- 没有规定每个块对应 `Request`/`Notification`/`Response`/`MediaFrame` 中的哪个类型；media 头还带有二进制尾巴，不能当作独立消息。
- §2 说“与 `wire.rs` 的夹具逐字节比对”，§8.1 又说“被抽成夹具”。如果 xtask 在运行时从 md 生成夹具，那么“改文档不改测试 → 红”永远不可能成立（改文档就改了夹具）；如果 `wire.rs` 是手写常量，文档才是被检查对象——需要明确选后者。
- Windows 工作区 `core.autocrlf=true`，且没有 `.gitattributes`；下次 checkout 会把 md 变成 CRLF，逐字节比较会因行尾 `\r` 全红。

建议：把代码围栏改成带类型的标记（` ```json request` / ` ```json response` / ` ```json notification` / ` ```json media-frame-header`），规定 `wire.rs` 持有期望字节、xtask 只做比较；抽取时统一 `trim_end_matches('\r')`；加 `.gitattributes`（`*.md text eol=lf`）或在门禁里显式按 LF 读。测试：`xtask ipc --self-test` 故意改一个 fixture 字节，断言红；再在 CRLF 检出上跑一次，断言仍绿。

### S5. 门禁 2/3 在代码侧没有 source of truth（P2，置信度 0.9）

`Call::method()` 在文档里没有声明；lane 只存在于 markdown 表（`ipc.md:236`），Rust 侧没有 `Lane` 类型。xtask 若要检查“method() 与 rename 一一对应”和“每个变体都有 lane”，只能文本解析枚举，脆弱且会漏掉 `match` 分支与表不一致。重命名时 gate 2 也不会红（rename 和表同步改就一致），无法支撑 §3 的“永久契约”。

建议：在 `wire` 里加 `pub enum Lane { Prompt, Slow, Operation, Stream }`，`impl Call { pub fn method(&self) -> &'static str; pub fn lane(&self) -> Lane; }` 用穷尽 `match`（编译器强制覆盖所有变体），再加一个 `const METHODS: &[(&str, Lane)]` 用于测试与文档表比对。测试：`methods_match_renames`、`every_variant_has_lane`（这两个是普通单测，不需要 xtask 解析源码）。

### S6. 门禁 4 的“每个 params 结构体”范围不清，且 unit 变体的 params 严格性没测（P2，置信度 0.9）

`ipc.md:162/237` 只约束顶层 params 结构体；`Encode`、`camera`、`imu`、`encoders`、`ConfigPatch` 等嵌套对象如果漏了 `deny_unknown_fields`，`"encode":{"bogus":1}` 会被静默接受。探针确认嵌套结构需要单独 deny，而且直接 derive 的 unit 变体会拒绝 map/array、却接受 `params:null`；§8.6 只测“线上没有 params”，没有测“收到非 null params 要报 INVALID_PARAMS”。

建议：把规则写成“从 params 递归可达的所有结构体都必须 deny_unknown_fields”；增加行为测试：每个嵌套层级各加一个未知字段，断言 `INVALID_PARAMS` 且 `data.field` 是 `encode.bogus` 形式；增加 `unit_variant_non_null_params_rejected` 测试（Envelope 路径必须显式检查，不能因为是 unit 就忽略 params）。

### S7. 门禁 6 要求的 unit/bare 夹具在 §2 不存在（P2，置信度 0.9）

`ipc.md:240-241` 要求 `record.status` 与 `record.get` 各一个往返夹具，但 §2 只有 hello/start/stop/两个通知/两个响应/media 头。夹具不在文档里，§8.1 的“所有 json 块”就覆盖不到它们；要么把夹具写进 §2（推荐），要么在 wire.rs 里手写并在 §8.6 明确“不要求出现在文档中”，二者选一。

测试：`unit_variant_fixture`、`scalar_params_fixture` 通过；否则门禁 6 自己就是红的。

### S8. 门禁 8 不可执行：没有 CHANGELOG，也没有基线能检测“破坏性改名”（P2，置信度 0.95）

`ipc.md:92/243` 要求 `API_VERSION` 变更同时改 `CHANGELOG` 的 `ipc:` 段，但仓库根目录没有 `CHANGELOG`（grep 只命中 ipc.md 自己），文件路径/段落格式也未定义；xtask 无法判断“这次 rename 是破坏性变更”。gate 2 对 rename 不敏感（表跟着改就一致）。

建议：新建 `CHANGELOG.md` 并规定 `## ipc` 段；在仓库里放一份 `action-ipc/contract-v<API_VERSION>.json`（方法名 + params 字段的冻结快照），xtask 对比方法集合/参数名，集合变化且 `API_VERSION` 没变 → 红；版本变了但没有 CHANGELOG 条目 → 红。测试：`xtask ipc` 的负向用例——只改一个 `#[serde(rename)]` 不改版本，断言红；改版本但不改 CHANGELOG，断言红。

### S9. `Event` 的前向兼容没有规则；未知通知会让旧客户端整行解析失败（P2，置信度 0.85）

`ipc.md:164-174` 只定义了两种事件；版本规则（`ipc.md:189`）只谈调用，不谈通知。新服务端加一个 `record.stopped` 通知时，旧客户端的 `Notification` 反序列化会得到 `unknown variant`，如果实现按行 `?` 处理，可能断开整个订阅。文档应规定：客户端先读 `method` 字符串，未知通知**忽略并继续**（与 `decode` 的两步法同理），且不得因此断开连接。

测试：`unknown_notification_is_ignored`：发送 `{"jsonrpc":"2.0","method":"record.future","params":{}}`，断言客户端仍能处理下一帧；以及 `decode_notification_unknown_method` 的行为断言。

### S10. `Operation` 的“同一连接串行”与 `Prompt` 的 500 ms 超时冲突（P2，置信度 0.85）

`ipc.md:199` 写 Operation “同一连接串行”，而 `record.status` 是 Prompt 500 ms。若 `uid` 用一条连接发 stop（可长达 120 s），随后在同一连接轮询 status，status 会排队并超时；文档没有说 Prompt 可以插队，也没有说 uid 必须为不同 lane 开不同连接。实际实现者会各自决定。

建议：要么明确“只有 Operation 之间串行，Prompt/Slow 可以并发处理”（服务端为每条连接维护请求队列 + 独立执行），要么明确“客户端必须为 Operation 使用独立连接，且 status 轮询走另一条连接”。测试：`status_during_stop_same_connection`：注入慢 stop，断言 status 在 500 ms 内返回；否则文档应把 uid 的连接策略写死。

### S11. interfaces 的 1440p 白名单没有对应的 fps 上限（P2，置信度 0.9）

`interfaces.md:210` 的 `encode.geom` 白名单包含 `2560×1440`，但 `ENC_FPS_CEIL_*` 只有 5MP/1080p/720p（`interfaces.md:66-68`），`interfaces.md:220` 又要求 `fps ≤ ceiling`。1440p 的校验无法实现，`record.start`/配置校验会落在未定义分支。

建议：补 `ENC_FPS_CEIL_1440P`（用实测值；没有实测就标注 `UNVERIFIED` 并暂不允许该分辨率），或从白名单删除 1440p。测试：`config_rejects_1440p60` 或 `config_accepts_1440p30`，依据实测值。

### S12. result schema 大量使用“…”和缩写，无法生成类型和 fixture（P2，置信度 0.9）

`ipc.md:146` 的 `hello` result（`{api_version, server, build, pid}` 无类型）、`:154` 的 `device.info`（`{encoders:{h264_fps_1080p,…}}` 字面省略号、`sensor:{w,h,fps}` 与 `record.status` 的 `{width,height,fps}` 命名不一致）、`:155-156` 的 `device.config.get/set`（“合并后有效配置”没有类型）都不可直接实现。`record.status` 的 `last_error?` 类型、`rec_id?` 的 null/省略语义也未定义。

建议：为每个 result 定义结构体（可以在 `action-ipc::results`），统一字段名（`width/height`，不用 `w/h`），把 `…` 换成明确字段；至少给 hello/status/device.info/config.get 各一条 fixture。测试：`result_schema_fixtures`；如果字段名两端不一致，fixture 会直接红。

### S13. `record.list`/`record.get`/`record.delete` 的边界语义缺失（P2，置信度 0.85）

- 不存在的 `rec_id` 返回什么？错误表没有 `NOT_FOUND`，只能落到 `E_STORAGE` 或 `INVALID_PARAMS`。
- `record.list` 的排序、`before` 是否排他、`next` 何时为 `None`、正在录制的目录是否出现在列表里（`complete=false` 还是完全不出现），都没有写。
- `record.delete` 的 `confirm:false` 是 no-op 还是 `INVALID_PARAMS`？删除正在录制/stopping 的记录怎么办？
- `record.get` 正在录制的记录返回部分摘要还是 `NOT_FOUND`？

建议：加一个 `E_NOT_FOUND`（或用 `INVALID_PARAMS` + `data.field:"rec_id"`，但要写死）；明确列表排序（按 `started_at` 倒序）、`before` 排他、`next = 最后一条.rec_id`；规定 in-progress 记录不出现在 list/get，delete 对 in-progress 返回 `E_STATE`；`confirm:false` 返回 `INVALID_PARAMS` 或 `E_STATE`（选一个）。测试：`get_missing_rec`、`delete_missing_rec`、`list_ordering_and_next`、`delete_in_progress`。

### S14. 非方法名的枚举值没有钉死线上字符串（P2，置信度 0.85）

`ipc.md` 只规定了 method 名的 `#[serde(rename)]`，但 `codec:"h264"|"h265"`、`format:"nv12"`、`state:"recording"`、`record.event.class:"E_STORAGE"` 都是线上值。`Codec` 若直接 derive，默认是 `"H264"`/`"H265"`；`PixelFormat::Nv12` 默认是 `"Nv12"`。fixture 只覆盖了 `"h264"` 和 `"nv12"` 各一次，start 的 fixture 会暴露 codec，format 只在 media 头出现；没有测试覆盖 `h265`/state/class。

建议：在 §3 增加一张 wire 值表，并对每个枚举写 `#[serde(rename_all = "...")]` 或逐变体 `rename`；加 `wire_value_table` 测试（每个枚举的每个变体都序列化成表里的字符串）。

### S15. 标定交接的 flock 语义在 IPC 上不可表达（P2，置信度 0.8）

`ipc.md:157-158` 说 `device.calibrate.enter` “actiond 释放硬件，锁交给 actioncal”，但 `architecture.md:58` 的 `device.lock` 是 `flock`：flock 属于打开的文件描述，不能通过一条 JSON-RPC 调用“交给”另一个进程；要么 actiond 关闭自己的 fd 让 actioncal 去抢（有竞态窗口），要么通过 `SCM_RIGHTS` 传 fd（文档没提）。另外 enter/leave 在 recording/stopping 时的错误、leave 的重开失败（`E_HW`）没有定义；`record.subscribe` 在标定期间是否暂停推送也没说。

建议：把交接写成可执行的协议：`enter` 返回后 actiond 必须已关闭设备与 `device.lock` fd；actioncal 自行 `flock(LOCK_EX)` 成功后才开始；`leave` 由 actiond 重新打开设备并重新 `flock`，失败返回 `E_HW`/`E_CALIB`；规定 enter 在非 idle 状态返回 `E_STATE`。测试：`calibrate_enter_closes_devices`（mock fd 断言 close 次数）、`calibrate_leave_reopens`、`calibrate_enter_while_recording_is_E_STATE`。

### S16. media 帧的时间域、旋转/letterbox 与 `payload_len` 校验（P2，置信度 0.8）

`ipc.md:86` 的 `pts_ns` 没有说明时钟域（interfaces 的 `MonoTime` 是 `CLOCK_MONOTONIC`，但线上没有写）；480×800 是从 2592×1944 横屏缩放来的，`interfaces.md:211` 说“可 letterbox”，但帧头没有旋转/letterbox 字段，UI 只能硬编码；`payload_len` 没有上限和形状校验（见 B5 的建议 4）。

建议：写明 `pts_ns = CLOCK_MONOTONIC`，与录制时间戳同源；定义固定的旋转/缩放约定（或加 `rotation: 0|90|180|270`、`letterbox:{x,y,w,h}`）；规定 `payload_len == stride*height*3/2` 且有一个合理上限（例如 16 MiB），否则断开并记 `E_IPC`。测试：`frame_header_pts_clock` 文档检查 + `frame_payload_len_mismatch_disconnects`。

### S17. interfaces 的门禁描述已过期：仍写“§8 六条”（P2，置信度 0.99）

`interfaces.md:267` 写 `xtask ipc` 覆盖“ipc.md §8 六条”，但当前 ipc.md §8 已是 **8 条**（2fe1ef2 新增了 unit/裸值夹具和错误响应无 result）。加上 `gates.md` 仍是“待写”，README 对门禁的描述与实际条目不一致，实现者不知道以哪份为准。

建议：改成“ipc.md §8 全部条目（当前 8 条）”，并让 `gates.md` 引用 ipc.md 的条目编号而不是复制数量。测试：`xtask docs` 增加一条“§8 条目数在 ipc.md 与 interfaces.md 一致”的检查（或干脆删除数量表述）。

### S18. 门禁的负向验证没有载体

`ipc.md:245` 要求每条门禁都有一次“故意让它红”的记录，但 `gates.md` 未写，也没有任何测试会检查“负向验证存在”。建议把负向验证做成 `xtask ipc --self-test`：在临时目录里变异一个 fixture/删除一个 `deny_unknown_fields`，断言门禁返回非零，并把结果写进 CI 日志；这样“红过”是可重复的，而不是口头承诺。

---

## 3. Nit

1. `Request.jsonrpc` 没有校验，`jsonrpc:"1.0"` 会被接受并回显（探针实测）；要么在 `decode` 里检查并返回 `-32600`，要么明确“字段只用于展示”。
2. `id: null` 被 `Id` untagged 枚举拒绝；JSON-RPC 2.0 允许 null（不推荐）。文档说“number 或 string”是刻意选择，建议补一句“null 视为 INVALID_REQUEST”，别让实现者以为漏了。
3. `record.event` 同时带 `class` 和 `code`，两者必须一致；建议 `code` 为权威，`class` 由 code 派生（或删除 class），避免 drift。
4. `record.progress` 的推送频率、是否在 stop 前推最后一条、是否有时间戳，都没有写；UI 做平滑显示时需要。
5. `record.get` 的 `RecId` 格式（`YYYYMMDD-HHMMSS-ssss`）只在 fixture 里出现，建议给出正则和校验失败的错误码。
6. `system.log.tail` 没有说明日志来源（journald 还是文件）、轮转、`lines:u16` 超过可用行数时的行为。
7. `system.shutdown` 只靠 `video` 组权限和 `confirm:bool`；“先停录”如果失败（`E_STORAGE`）是否仍然关机、响应是否在进程退出前 flush，都没有写。
8. `record.stop` 最长 120 s，但没有中间进度通知；UI 只能转圈。可以规定 stop 也发 `record.progress`（带 `stopping:true`）或 `record.stop.progress`。
9. `hello` 的 `server/build/pid` 类型未定义；`pid` 在 systemd 环境下是主进程还是 worker 也要说明（如果只是诊断，用 String 更稳）。
10. `device.info` 用 `{w,h,fps}`，`record.status` 用 `{width,height,fps}`；统一字段名，否则客户端要写两套解析。
11. `semver` 被列为 action-ipc 的允许依赖，但版本规则用的是 `u32`；要么删掉 semver，要么在 `hello.server` 里用 semver 字符串并说明兼容规则。
12. 文档说“所有 params 结构体 deny_unknown_fields”，但 `Request` 有 flatten，顶层未知字段无法 deny（探针实测被静默接受）；这个限制应写进 §1，并明确 Envelope 负责顶层严格性。
13. `Response` 允许 `result` 和 `error` 同时为 `None`（探针输出 `{"jsonrpc":"2.0","id":9}`）；建议加构造器/不变量，禁止这种响应被发送（B4 的一部分）。
14. `record.delete`/`system.shutdown` 的 `confirm` 在机器 API 里语义偏弱；如果只是防误触，建议命名为 `force` 或在文档里说明为何需要它。
15. `docs/design/README.md:11` 的 record-format 仍“待写”，而 ipc.md 已经把它当作契约依赖；README 的状态列应同步（或明确“冻结前提”）。

---

## 4. 如果照现在这样冻结，最可能在第几步炸

**第 1 步（`action-ipc` crate + 第一次 `cargo xtask ipc`）就会炸，而不是等到设备上。** 因为 §2 的 `record.start` fixture 按文档类型会多出 `"note":null`，`media.frame` fixture 会被 `Value` 的 BTreeMap 重排；门禁 1 是红的。开发者的第一反应通常是“给 note 加 skip”或“把 fixture 改成 null”，前者是文档没写的隐式规则，后者会改掉线上样例；无论选哪个，都在冻结当天修改契约。同一时刻，`ConfigPatch { value: toml::Value }` 会逼着 `action-ipc` 依赖 `toml`，而 `xtask arch` 又禁止这个依赖——**两条门禁不可能同时绿**，这是第一个硬返工点。

**第 2 步（actiond 状态机 + uid）**：`record.status.state` 没有取值表，`record.stop` 的 120 s 期间 `record.start`/`record.status` 行为未定义。服务端可能返回 `"stopping"`，UI 只认 `"idle"/"recording"`，于是停止期间界面显示 idle 或直接报未知状态；同一连接上的 status 轮询还可能排在 stop 后面超时。磁盘满自动停止时只发一条自由文本 `record.event`，订阅者拿不到最终 `IndexSummary`，UI 会一直停在“recording”。

**第 3 步（预览）**：§3.2 没有任何 media 方法，`uid` 和 `actiond` 会各自发明“开始推流”的约定（有人用连接即推，有人发一个文档里不存在的方法），`id:7` 没有匹配请求；关闭语义、慢客户端丢帧、payload 边界都没有定义。这个问题不会在 host 单测里暴露，第一次上板联调才会发现两端对不上。

**第 4 步（标定交接）**：`device.calibrate.enter` 说“锁交给 actioncal”，但 flock 不能通过 JSON-RPC 传递；actioncal 现在虽然在 §7 的客户端表里，但没有说明它如何拿到锁、actiond 何时关闭 fd、leave 重开失败怎么办。实现者很可能写出“enter 后 actioncal 永远等不到锁”或“actiond 没关设备，actioncal 打开失败”的版本。

**第 5 步（门禁收口）**：`Call::method()` 和 lane 没有代码侧来源，xtask 只能文本解析；CHANGELOG 不存在，rename 不会被检测。等到发现 `codec` 默认序列化成 `"H264"`（而不是 fixture 里的 `"h264"`）时，方法名/枚举值的“永久契约”已经没有任何机器保护，只能靠 review 记性。

**最小冻结前清单**：先修 B1/B2（否则门禁 1 永远红）→ B6（否则 arch 与 ipc 门禁互斥）→ B3/B4/B5（否则两端各写一套）→ B7/B8（否则核心录制路径和 UI 状态机没有类型和状态定义）→ 再落 S5/S8（把 method/lane/版本基线放进代码，门禁才真的可执行）。

---

**总体判定：incorrect。** 8 条阻断项中有 6 条是文档内部或文档之间的可证明矛盾/缺口，2 条是当前 fixture 直接红；其中 B1/B2/B6 会在实现的第一天就阻断门禁，B3/B4/B5/B7/B8 会在 M1 联调时造成两端返工。Serde 表示本身没有“隐性错误”：unit 变体、flatten、错误响应的当前写法经探针验证是正确的（0.1/0.2），问题集中在 fixture 的可满足性、类型化结果、media 生命周期和门禁的 source of truth。置信度 0.92。

