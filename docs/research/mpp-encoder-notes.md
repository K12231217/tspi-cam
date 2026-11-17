# MPP 硬件编码 bring-up（2026-11-16）：走到哪、卡在哪

- 板：`k@192.168.31.129`（Ubuntu 20.04.6，kernel 4.19.232 aarch64）
- 库：`/usr/lib/aarch64-linux-gnu/librockchip_mpp.so.0`（1.5.0，`rkisp_v5` 同时代 BSP）
- 结论：**dlopen 与 encode/get_packet 链路已打通，H.264 码流可出；但 `MPP_ENC_SET_CFG` 被拒，
  实际按默认 1280x720 编码**。编码器因此没有接进管线，`action-core::encode` 只保留契约类型。

## 已确认可用

1. `dlopen("librockchip_mpp.so.1")` + `dlsym` 可行（**必须动态链接**；musl static 报
   "Dynamic loading not supported"，见下节）。
2. `MppApi` 表结构：`size = 184`（8 + 14×8 + 16×4），`encode_put_frame` 偏移 40、
   `encode_get_packet` 48、`control` 112，实测与 `rk_mpi.h` 声明顺序一致。
3. `mpp_create` / `mpp_init(MPP_CTX_ENC, MPP_VIDEO_CodingAVC)` / `mpp_enc_cfg_init` /
   `mpp_enc_cfg_set_s32` / `mpp_enc_cfg_get_s32` 全部返回 0；cfg 读回值正确
   （`prep:width=1920`、`prep:height=1080`、`codec:type=7`、`rc:bps_target=10000000`）。
4. `mpp_buffer_group_get(DRM)`、`mpp_buffer_get_with_tag`、`mpp_buffer_get_ptr_with_caller`、
   `mpp_buffer_put_with_caller` 可用；帧数据 copy 进 MPP buffer 后 `encode_put_frame` 返回 0。
5. `encode_get_packet` 每个输入帧产 1 个 packet，`mpp_packet_get_pos/length/pts` 可读；
   **本库在队列空时返回 `ret=0` 且 `packet=NULL`**（不是 EAGAIN），不解引用空指针即可。
6. 30 帧 1080p 输入在板上跑到 28.17 fps（含采集），包数 31，码流 `ffprobe` 可识别为 h264；
   MPP 日志显示内部按 `1280x720` 走 RGA（`RGA_BLIT fail: Invalid argument` 逐帧出现）。

## 卡点（未解决）

- `control(ctx, MPP_ENC_SET_CFG, cfg)`：
  - `0x00320001` 在最小配置（prep:width/height/format + codec:type）与完整配置下都返回
    `-6 = MPP_ERR_VALUE`；
  - `0x00320002..0x00320009` 返回 0，但**不会改变编码分辨率**；
  - 输出 SPS 固定 1280x720（`ffmpeg` 抽帧确认 1280x720），与 cfg 读回的 1920x1080 矛盾。
- 已排除：cfg 指针为空、键名错误（`strings librockchip_mpp.so` 确认 `prep:*`/`rc:*`/`codec:type`
  名称；`rc:fps_in_denorm` 不是 `denom`）、api 表偏移错误（put/get 均正常）、
  缓冲区类型（DRM 组创建成功）。
- 未排除（下一步，按顺序）：
  1. 该库的 `MppEncCfg` 需要 `mpp_enc_cfg_set_st` 设置 `prep:format` 等结构体而非 s32；
  2. SET_CFG 之前必须先用**废弃结构体命令**（`MPP_ENC_SET_PREP_CFG` 0x00320003 等）建立
     基础 prep，再 SET_CFG 覆盖；
  3. 库与头文件版本不匹配（头是 1.5.0，`.so.0` 可能是更早/更晚的 ABI）；
     对照方法：用板上 `gst-inspect-1.0 rockchipmpp` 的 gst 插件源码或 `mpi_enc_test`
     样例的 cfg 设置顺序复刻。
  4. 校验失败原因可能被 `-6` 吞掉：MPP 有 `mpp_log` 级别环境变量
     （`MPP_LOG_LEVEL`/`mpp_log_set_level`），下次先开 debug 日志再复现。

## 交叉编译/运行（已打通，值得复用）

- WSL Ubuntu 24.04 的 glibc 2.39 与板端 2.31 不兼容；musl static 不能 dlopen。
- 运行方式：把 WSL 的 aarch64 glibc 2.39 运行库打到板上，用显式 loader 启动：
  ```text
  /tmp/g239/ld-linux-aarch64.so.1 --library-path /tmp/g239 /tmp/app ...
  ```
  需要 `libc.so.6`、`libm.so.6`、`libpthread.so.0`、`libdl.so.2`、`librt.so.1`、
  `libgcc_s.so.1`、`ld-linux-aarch64.so.1`（全部来自 `/usr/aarch64-linux-gnu/lib`）。
- 这是**探针验证**手段，不是发布方案；正式部署仍需在 Ubuntu 20.04 环境（容器/CI）构建，
  或改用板端 musl 静态 + 纯 Rust 实现（MPP 只能动态加载，故最终大概率是 20.04 构建环境）。
- 尝试过 `libc6-dev 2.31 sysroot + aarch64-linux-gnu-gcc`，链接阶段被 rustc 的 lld/flavor
  与 libgcc 依赖卡住，未成功；记录在此避免重复踩坑。

## 复现命令（当时）

```bash
# 交叉构建（WSL）
cargo build --release --target aarch64-unknown-linux-gnu --example encode_probe
# 板上
/tmp/g239/ld-linux-aarch64.so.1 --library-path /tmp/g239 \
  /tmp/encode_probe239 /dev/video0 1920 1080 30 10000 30 /tmp/out.h264
ffprobe -v error -show_entries stream=codec_name,width,height -of default=nw=1 /tmp/out.h264
# -> h264, 1280x720（应为 1920x1080）
```
