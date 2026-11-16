# tspi-cam 设计文档索引

> 组织方式照抄 microduck：**一个机制一页，谁拥有谁负责**。事实只写在该机制的页里，
> 其他页只写一句 + 链接；两页冲突时，不拥有该机制的那页是 bug。

| 页面 | 拥有的机制 | 状态 |
|---|---|---|
| [architecture.md](architecture.md) | 服务划分、socket 拓扑、状态归属、谁在哪个组（安装/切换/udev 归 deploy.md） | 已定（2026-11-16，rev3） |
| [ipc.md](ipc.md) | 线上契约：JSON-RPC 2.0 NDJSON、`Call`/`Event`、方法名、错误码、lane/版本规则、socket 权限表 | 已定（2026-11-16，rev3） |
| [interfaces.md](interfaces.md) | crate 边界、Rust trait 签名、Buffer Pool 生命周期、**stride/vstride 定义** | 已定（2026-11-16，rev3） |
| [record-format.md](record-format.md) | `rec-*/` 布局、`IndexSummary`/`index.json` 字段、完整性判定、状态机 | 已定（2026-11-16） |
| timebase.md | 五个时间点、时钟域、MIPI 时间戳语义核查、离线对齐 | 待写 |
| capture-encode.md | V4L2/MPP 实现细节：dmabuf 路线、线程/队列、帧率上限（stride 定义归 interfaces） | 待写（实测数据见 `../architecture.md` §16 H9–H16） |
| ui.md | 显示/触摸：Xorg + libmali EGL + egui，预览变换与触摸映射 | 待写（决策与实测见 `../research/ui-stack-decision.md`） |
| calibration.md | `actioncal` 与 `actiond` 的设备锁、交接流程、时间同步/空间对齐方法论 | 待写 |
| [deploy.md](deploy.md) | 交叉编译、安装布局、systemd 单元与切换、udev、组/账号的安装动作、回滚与验收检查表 | 已定（2026-11-16） |
| gates.md | `cargo xtask` 门禁：arch/ipc/test/docs/smoke/size 与负向验证方法 | 待写 |

旧的 `../architecture.md`（v1.2，中文）保留为**历史与实测记录**（H1–H16、队列/编码/UI 的板上数据）；
系统架构以本目录为准，两者冲突时以 `docs/design/` 为准。

## 参考

- microduck（同龄 RK3566、Rust、多守护进程）：`docs/design/architecture.md`、`duck-ipc-proto`、
  `mediad`。采用它的**服务划分 + 单一 JSON-RPC 契约 + 每服务一个 socket + 配置分层**，
  不采用它的 WebRTC/更新器/BLE（本项目 Demo 范围不需要）。
- 本机知识库：`Linux Vision`（数据流 15 章）、`姿态估计算法/外部真值标定方法论`、
  `Rust 学习/契约层与机器门禁`、`嵌入式架构/项目代码框架`。
