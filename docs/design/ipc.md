# tspi-cam IPC 契约

- 状态：v1（2026-11-16）· 拥有者：`runtime/action-ipc`（唯一实现这份契约的 crate）
- 形态照抄 microduck 的 `duck-ipc-proto`：**JSON-RPC 2.0，一行一个对象（NDJSON），每服务一个 Unix socket**。
- 该 crate 只依赖 `serde` / `serde_json` / `semver`：所有服务（包括坏掉时还要能用的 `actionctl`）
  都编译它，不允许引入 tokio、http、crypto、图像库。

## 1. 帧格式

```text
→ {"jsonrpc":"2.0","id":1,"method":"record.start","params":{...}}
← {"jsonrpc":"2.0","method":"record.progress","params":{...}}      ← 通知（无 id）
← {"jsonrpc":"2.0","id":1,"result":{"rec_id":"20261116-141230-3f2a"}}
```

- 分帧：单个 `\n`。一个连接上请求可并发，靠 `id` 匹配（`id`: number 或 string）。
- 进度/事件用**通知**（无 `id`）推送，客户端断线重连后重新 `Subscribe` 即可继续收。
- 参数严格：所有 `params` 结构体 `#[serde(deny_unknown_fields)]`——**多一个字段就报
  `INVALID_PARAMS` 并指名字段**，这是防止"旧客户端以为改了一个不存在的参数"的唯一机制。

## 2. 唯一契约：`Call` 枚举

方法名与参数在类型上成对绑定，**不存在"方法 A + 参数 B"的发送方式**（microduck 同款做法）：

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum Call {
    /// 每条连接的第一个调用。
    Hello(HelloParams),

    // ── record.* ─────────────────────────────────────────────
    RecordStatus,                       // 快照：状态、已录时长、帧/样本计数、磁盘余量
    RecordStart(StartParams),           // 进入 Starting；参数覆盖本次录制（分辨率/码率/编码器）
    RecordStop(StopParams),             // 走 Drain→Flush→Finalize，返回 IndexSummary
    RecordList(ListParams),             // 分页列 rec-*
    RecordGet(RecId),                   // 读 index.json
    RecordDelete(RecId),                // 删除（需 confirm=true）
    RecordSubscribe,                    // 之后持续收到 record.progress / record.event

    // ── device.* ─────────────────────────────────────────────
    DeviceInfo,                         // 相机/IMU/编码器能力 + 实测上限（只读）
    DeviceConfigGet,                    // 合并后的有效配置（基础层 + 用户层）
    DeviceConfigSet(ConfigPatch),       // 只写用户层；返回新有效配置
    DeviceCalibrateEnter,               // 取 device.lock 交给 actioncal；actiond 释放硬件
    DeviceCalibrateLeave,               // actioncal 结束，actiond 重新取锁开设备

    // ── system.* ─────────────────────────────────────────────
    SystemLogTail(LogParams),           // 最近 N 行 journald（排障用，不落盘）
    SystemShutdown(ShutdownParams),     // 停录后关机（confirm=true）
}
```

不是 `Call` 的特例（microduck 也这样处理原始帧）：

- `media.frame`：只在 `/run/tspi-cam/media.sock` 上，先回 JSON-RPC 头，紧接着是头里声明长度的
  二进制负载（NV12 平面）。**永不进入控制通道**，也永远不是 `Call`。
- `record.progress` / `record.event`：只作为通知出现，不是请求。

## 3. 方法与 lane

每条方法声明一个 lane，`actionctl` 与 `uid` 按 lane 选超时；服务端按 lane 决定是否允许并发。

| lane | 语义 | 超时 | 方法 |
|---|---|---|---|
| `Prompt` | 纯内存查表 | 500 ms | `RecordStatus`、`DeviceInfo`、`DeviceConfigGet` |
| `Slow` | 读盘/扫目录 | 5 s | `RecordList`、`RecordGet`、`SystemLogTail` |
| `Operation` | 会改变状态，可能几十秒 | 120 s | `RecordStart`、`RecordStop`、`RecordDelete`、`DeviceConfigSet`、`DeviceCalibrateEnter/Leave`、`SystemShutdown` |
| `Stream` | 永不返回，写通知直到对端断开 | — | `RecordSubscribe`、`media.frame` |

- `RecordStart` 在 `Starting` 阶段允许被 `RecordStop` 取消；`Stopping` 期间新的 `RecordStart/Stop`
  返回 `BUSY_STOPPING`。
- 同一连接上 `Operation` 串行；不同连接之间由 `actiond` 状态机裁决。

## 4. 版本规则（照抄 microduck 的结论）

```rust
pub const API_VERSION: u32 = 1;             // 任何不兼容改动都 +1
pub struct HelloParams  { pub api_version: u32, pub client: String }
pub struct HelloResult  { pub api_version: u32, pub server: String, pub build: String, pub pid: u32 }
```

- **版本不同不拒绝任何调用。** 只记录、只在 `HelloResult` 里回报。真正会失败的只有两种情况：
  方法不存在（`METHOD_NOT_FOUND`，报方法名）或参数形状变了（`INVALID_PARAMS`，报字段名）。
- 理由（microduck 实测踩过的坑）：在握手上拒绝会让"能服务的调用"一起失败，包括
  `update apply` —— 而版本偏差本来就该由那一条调用自己失败。
- 协议"只会向前兼容不了"：旧 daemon 不会拒绝它一直忽略的字段，所以升版本号仍然必要；
  所有二进制总是一起发布（安装器保证）。

## 5. 错误码

`code` 是稳定的数字，`data.class` 是给 UI 分类的字符串，`message` 给人看。

| 数字 | class | 含义 | 典型来源 |
|---|---|---|---|
| -32700 | `PARSE` | 不是合法 JSON | 解码 |
| -32601 | `METHOD_NOT_FOUND` | 方法不存在（data 里带方法名） | 分发 |
| -32602 | `INVALID_PARAMS` | 参数不合法（data 里带字段名） | serde 严格模式 |
| 1001 | `E_STATE` | 非法状态转移（`ALREADY_RECORDING`/`NOT_RECORDING`/`BUSY_STOPPING`） | 状态机 |
| 1002 | `E_HW` | 设备打不开/丢失/编码器失败 | 采集/编码 |
| 1003 | `E_STORAGE` | 磁盘余量不足、写盘失败 | 写盘/保护 |
| 1004 | `E_CFG` | 配置不合法（分辨率不在白名单等） | 配置 |
| 1005 | `E_IPC` | socket/权限/对端断开 | 传输 |
| 1006 | `E_CALIB` | 标定文件缺失/损坏/版本不符 | 标定 |

错误码随 `record.event` 与 `index.json.errors` 落盘，**不允许只打日志**。

## 6. Socket 与权限

| socket | 模式 | 所有者 | 谁连 |
|---|---|---|---|
| `/run/tspi-cam/record.sock` | 0660 | `root:video`（`actiond` bind 后 `fchown`） | `uid`、`actionctl` |
| `/run/tspi-cam/media.sock` | 0660 | `root:video` | `uid`（预览） |

- `actiond` 启动时先建 `/run/tspi-cam/`（systemd `RuntimeDirectory`），再 bind、`fchown`、`chmod`，
  最后才 `Hello` 可用。**权限写死在 daemon 里，不靠 umask。**
- 客户端连不上时错误必须是 `E_IPC` + 路径 + `strerror`，不允许静默重试到超时。

## 7. 兼容性检查清单（CI 门禁）

`cargo xtask ipc` 检查：

1. `method` 常量与 `Call::method()` 一一对应，无孤儿方法名；
2. 每个 `Call` 变体的 `params` 结构体都带 `deny_unknown_fields`；
3. 每个方法在 lane 表里有且只有一个 lane；
4. `API_VERSION` 变更必须同时改 `CHANGELOG` 的 `ipc:` 段（缺失则门禁红）。

这四条都是"能被机器判定的契约"，不靠 review。
