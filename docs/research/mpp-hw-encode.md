# MPP 硬件编码调研：RK3566 / Ubuntu 20.04 / MPP 1.5.0

> 范围：H.264/H.265 1080p30 实时编码，供 Rust 项目调用。
> 结论级别：`VERIFIED` = 有仓库源码/官方文档/板上实测依据；`INFERENCE` = 由已证实事实推导；`UNVERIFIED` = 未取得直接证据，必须由 spike 确认。

## 0. TL;DR

1. **RK3566 支持 H.264 1080p30 与 H.265 1080p30**。RK3566 的 VPU 规格为 H.264 编码 1080p60、H.265 编码 1080p100（Firefly ROC-RK3566-PC 规格表），1080p30 有 2× 以上余量。
2. 板上 `mppvideodec`/`mppjpegdec` 可见、`mpph264enc` 不可见，**很可能不是插件缺编码器，而是 `/dev/mpp_service` 权限为 `0600 root:root`**。MPP 插件先无条件注册解码器，再通过 `mpp_create`/`mpp_init` 探测编码器；探测失败时编码器被静默跳过。RK3566（Radxa Zero 3W）上已实测该行为。先做 `chmod`/udev + `strings` 验证，不要急着换技术栈。
3. **最省事且推荐**：GStreamer `mpph264enc`，Rust 侧用 `gstreamer-rs`。板上已有 GStreamer + MPP，插件大概率已包含编码器；修权限后即可零代码验证。1080p30 用 H.264 High + CBR 4–8 Mbps + `header-mode=each-idr`。
4. **备选**：直接 MPP C API + 薄 Rust FFI（`libloading` + bindgen 生成类型）。不要直接依赖 `yscv-video-mpp`（常量错误、缺 SPS/PPS、仅 H.264），也不要用 `rs-mpp`（只绑定了 `mpp_log`）。
5. **不推荐**：FFmpeg（Ubuntu 20.04 自带 4.2 没有 rkmpp 编码器，需自建 8.1+）、`libv4l-rkmpp`（Chromium VDA/VEA 专用）。
6. 最小 spike：`chmod 666 /dev/mpp_service` → `gst-inspect-1.0 mpph264enc` → `mpi_enc_test -w 1920 -h 1080 -t 7 -n 300 -v f` → 300 帧 GStreamer 管线。命令见 §6。

---

## 1. 事实基线

| 项目 | 已确认事实 | 证据 |
|---|---|---|
| SoC | RK3566，4×Cortex-A55，Mali-G52 | 板卡资料 |
| VPU 编码 | H.264 1080p60，H.265 1080p100（规格表）；概述页写 1080p60 H.264/H.265 | [Firefly ROC-RK3566-PC 规格](https://download.t-firefly.com/Spec/Mainboards/ROC-RK3566-PC_Specification_EN.pdf) |
| MPP | `librockchip_mpp.so.1.5.0`；包版本 `1.5.0-1+git20240717.5275a9a6` | 板上 `dpkg -l` / PPA build；[Launchpad](https://bugs.launchpad.net/~jjriek/+archive/ubuntu/rockchip-multimedia/+build/28666038) |
| MPP 设备 | `/dev/mpp_service` 存在；RK3566 BSP 内核通过它暴露 VPU，不是 V4L2 M2M | [microduck 实测](https://github.com/pollen-robotics/microduck/blob/main/docs/project/media-bringup.md) |
| GStreamer 插件 | 当前只看到 `mppvideodec`/`mppjpegdec`；源码树 `gstreamer-rockchip` 同时含 `mpph264enc`/`mpph265enc`/`mppjpegenc`/`mppvp8enc` | [JeffyCN/mirrors@gstreamer-rockchip](https://github.com/JeffyCN/mirrors/tree/gstreamer-rockchip/gst/rockchipmpp) |
| 板上编译 | 无 gcc，C 程序不能就地编译 | 任务约束 |
| GStreamer 版本 | Ubuntu 20.04 发行版为 1.16.x（`INFERENCE`，spike 用 `gst-inspect-1.0 --version` 确认） | Ubuntu 20.04 |
| 显示 | 480×800@60 HDMI，X11，无 Wayland 合成器 | 任务约束 |

---

## 2. 11 个仓库/资料对比

> 表中“RK3566/MPP 1.5.0 证据”只写可核查内容；没有直接证据就标 `UNVERIFIED`。

| # | 仓库/资料 | 角色 | H.264 / H.265 编码能力 | aarch64 / RK3566 / MPP 1.5.0 证据 | License | 结论 |
|---|---|---|---|---|---|---|
| 1 | [rockchip-linux/mpp](https://github.com/rockchip-linux/mpp) | 官方 C 库（MPI/MPP/HAL） | 编码 H.264、H.265、VP8、MJPEG；`mpp/hal/rkenc/h264e`、`h265e` | README 明列 RK3566；`mpi_enc_test` 为官方编码示例；MPP 1.5.0 即此库的 2024-07 快照 | **Apache-2.0**（`debian/copyright`）；仓库部分文件 `Apache-2.0 OR MIT` | **必选底层**；直接 FFI 或 GStreamer/FFmpeg 都经它 |
| 2 | [MPP 官方开发指南 + `test/mpi_enc_test.c`](https://github.com/rockchip-linux/mpp/blob/develop/doc/Rockchip_Developer_Guide_MPP_EN.md) | 官方文档/示例 | 文档给出 `MPP_CTX_ENC`、`MppEncCfg` 参数表、`MPP_ENC_GET_HDR_SYNC`、`encode_put_frame`/`encode_get_packet` 流程；示例含 1080p 30 帧编码，实测 47.72 fps（官方文档，平台未注明） | `mpi_enc_test -w 1920 -h 1080 -t 7 ...`；编码类型 AVC=7，HEVC=0x01000004 | Apache-2.0 / MIT | **API 序列的权威来源**；spike 直接跑它 |
| 3 | [JeffyCN/mirrors 分支 `gstreamer-rockchip`](https://github.com/JeffyCN/mirrors/tree/gstreamer-rockchip/gst/rockchipmpp) | GStreamer 插件（上游镜像） | `gstmpph264enc.c`、`gstmpph265enc.c`、`gstmppjpegenc.c`、`gstmppvp8enc.c` 全在树内 | 分支 `gstreamer-rockchip`；Launchpad 源码包 `1.14-4+git240423.4d8322c`（2024-04-23）；Firefly 文档给出 `mpph264enc`/`mpph265enc` 管线；2024-05 修复过 1080p H.265 卡顿 | **LGPL-2.1** | **首选接入层**；板上二进制若已含编码器，只差权限 |
| 4 | [Fruit-Pi/gstreamer-rockchip](https://github.com/Fruit-Pi/gstreamer-rockchip) | 可核查的源码镜像/fork | `gst/rockchipmpp/meson.build` 无条件编译 `gstmpph264enc.c`/`gstmpph265enc.c`；工程版本 1.14.4 | 与 #3 同源；`meson.build` 要求 GStreamer >= 1.14；Ubuntu 20.04 的 1.16 满足 | LGPL-2.1 | 用来读源码/自建；不要依赖某个个人 fork 的二进制 |
| 5 | [FFmpeg 官方 `libavcodec/rkmppenc.c`](https://ffmpeg.org/doxygen/trunk/rkmppenc_8c_source.html) | FFmpeg 官方编码器 | `h264_rkmpp`、`hevc_rkmpp`；支持 `AV_PIX_FMT_DRM_PRIME`、NV12、YUV420P；CBR/VBR/CQP | **FFmpeg 8.1 Changelog**：“Rockchip H.264/HEVC hardware encoder”；configure 要求 `rockchip_mpp >= 1.3.8` + `--enable-rkmpp` + libdrm；板上 1.5.0 满足 | LGPL-2.1+（不加 `--enable-gpl`） | 可行，但 **Ubuntu 20.04 自带 FFmpeg 4.2 不支持**，需自建 8.1+ |
| 6 | [nyanmisaka/ffmpeg-rockchip](https://github.com/nyanmisaka/ffmpeg-rockchip) | FFmpeg 第三方 fork | `h264_rkmpp`、`hevc_rkmpp`、`mjpeg_rkmpp`；RGA 滤镜；零拷贝 | README 明示目标 RK3588/3588s；BSP 内核 5.10/6.1；**未写 RK3566** | LGPLv3（README 要求 `--enable-version3`） | RK3566 支持 `UNVERIFIED`；不作为首选 |
| 7 | [JeffyCN/libv4l-rkmpp](https://github.com/JeffyCN/libv4l-rkmpp) | V4L2 → MPP 包装（Chromium VDA/VEA） | 有 `src/libv4l-rkmpp-enc.c`（VEA 编码） | README 明示“requires custom patches”“lots of chromium related hacks”；需伪造 `/dev/video-enc0`；非通用 API | LGPL-2.1 | **拒绝**；仅适合 Chromium 魔改栈 |
| 8 | [yscv-video-mpp](https://crates.io/crates/yscv-video-mpp) / [源码](https://github.com/enthropy7/YSCV/tree/main/crates/yscv-video-mpp) | Rust FFI 封装（0.2.0） | 文档称 H.264/HEVC；实现只有 `MppH264Encoder`（`MPP_VIDEO_CODING_AVC`） | `dlopen("librockchip_mpp.so")`，编译跨平台；但源码常量有误：`MPP_VIDEO_CODING_HEVC=0x10000001`（实际 `0x01000004`）、`MPP_FMT_YUV420P=1`（实际 4）；`rc:fps_in_denorm` 拼错（应为 `rc:fps_in_denom`）；未调用 `MPP_ENC_GET_HDR_SYNC` | Apache-2.0 | **不可直接依赖**；可读作反面参考/代码片段 |
| 9 | [rainbowflesh/rs-mpp](https://github.com/rainbowflesh/rs-mpp) | bindgen FFI | README 明示当前仅支持 `rk_type` 与 `mpp_log` | 未绑定 `mpp_create`/`mpp_init`/编码 API | “Other”（不明确） | **不可用** |
| 10 | [shum-elli/rknn_cam_rtsp](https://github.com/shum-elli/rknn_cam_rtsp) | RK3566 参考项目 | 640×480 H.264，MPP C API；含 `GetHeader()` 取 SPS/PPS 后喂 RTSP | README 明示 RK3566、`librockchip_mpp.so` aarch64、`MPP_VIDEO_CodingAVC`；代码演示 SPS/PPS 时序 | `UNVERIFIED`（仓库未标许可证） | **最佳调用序列参考**，但只到 640×480，不能证明 1080p30 |
| 11 | [rigaya/rkmppenc](https://github.com/rigaya/rkmppenc) | 编码性能/质量实验工具 | H.264/HEVC；CQP/VBR/CBR；MPP 编码 + 解码 + VPP | README 明示 MPP H.264/HEVC；示例在 RK3588；未写 RK3566 | **MIT** | 可用于交叉验证码流/性能；RK3566 `UNVERIFIED` |

### 2.1 关于 “gst-rkmpp 插件仓库”

- **没有找到可公开访问的 `gst-rkmpp` 仓库**：`github.com/rockchip-linux/gst-rkmpp` 返回 404，GitHub 仓库搜索 `gst-rkmpp` 为 0 条。其历史定位是 Chromium VDA/VEA 的 **decode-only** 元素集（`mppvideodec`/`mppjpegdec`），**`mpph264enc` 不在它里面**。该仓库的可用性/历史标 `UNVERIFIED`。
- `mpph264enc`/`mpph265enc` 所在的是 **`gstreamer-rockchip` 的 `gst/rockchipmpp`**，不是 `gst-rkmpp`：
  - 分支：`gstreamer-rockchip`（`JeffyCN/mirrors`）
  - 版本：Launchpad 源码包 `1.14-4+git240423.4d8322c`；工程 `version: 1.14.4`
  - 源码：`gstmpph264enc.c`、`gstmpph265enc.c`、`gstmppjpegenc.c`、`gstmppvp8enc.c`
- 因此，**不要去找 “gst-rkmpp 的编码分支”**；要装/修的是 `gstreamer-rockchip` 的 `rockchipmpp` 插件。

### 2.2 为什么 `gst-inspect` 只看到 `mppvideodec`/`mppjpegdec`

源码逻辑（`gstmppenc.c`）：

```c
gboolean gst_mpp_enc_supported(MppCodingType mpp_type)
{
    MppCtx mpp_ctx;
    MppApi *mpi;

    if (mpp_create(&mpp_ctx, &mpi))
        return FALSE;
    if (mpp_init(mpp_ctx, MPP_CTX_ENC, mpp_type)) {
        mpp_destroy(mpp_ctx);
        return FALSE;
    }
    mpp_destroy(mpp_ctx);
    return TRUE;
}
```

编码器元素只有在 `mpp_create` + `mpp_init(MPP_CTX_ENC, ...)` 成功后才注册。`/dev/mpp_service` 为 `0600 root:root` 时，非 root 进程打不开设备，`mpp_init` 失败，编码器被静默省略；解码器是无条件注册的，所以看起来像“插件只有解码器”。

RK3566（Radxa Zero 3W，MPP 1.5.0）上已实测：
- `/dev/mpp_service` 默认 `0600 root:root`；
- `mpi_enc_test` 非 root 运行会写出 **空文件并退出 0**（退出码不能作为证据，必须看文件大小）；
- `strings libgstrockchipmpp.so` 能看到 `mpph264enc`/`mpph265enc`，但 `gst-inspect` 只显示两个解码器；
- 加 udev 规则（`0660 root:video`）并把用户加入 `video` 后，`gst-inspect-1.0 mpph264enc` 正常。

来源：[microduck media-bringup](https://github.com/pollen-robotics/microduck/blob/main/docs/project/media-bringup.md)（RK3566 实测）。

---

## 3. 三个问题的直接回答

### 3.1 Ubuntu 20.04 + MPP 1.5.0 上，H.264/H.265 1080p30 是否支持？

**支持。** 分层结论：

| 层 | H.264 1080p30 | H.265 1080p30 | 证据 |
|---|---|---|---|
| RK3566 硬件 | ✅ 支持（规格 1080p60） | ✅ 支持（规格 1080p100） | Firefly 规格表；Rockchip RK3566 datasheet |
| MPP 1.5.0 用户态 | ✅ `MPP_VIDEO_CodingAVC` + `mpp/hal/rkenc/h264e` | ✅ `MPP_VIDEO_CodingHEVC` + `mpp/hal/rkenc/h265e` | MPP README 支持平台含 RK3566；MPP 开发指南第 3.4/3.5 节 |
| 板上实测 | ⚠️ 720p H.264 已在同 SoC（Radxa Zero 3W）实测；**1080p30 本板尚未测** | ⚠️ **本板 1080p30 尚未测** | microduck 实测 720p；1080p30 需 §6 spike |

关键证据：
- MPP 官方文档明确编码支持 **H.265 / H.264 / VP8 / MJPEG**，并给出 `mpi_enc_test -w 1920 -h 1080 -t 7 -i ... -o ...` 的 1080p 示例，示例日志为 `encode 30 frames time 628 ms ... fps 47.72`（官方文档未注明该次测试的 SoC，不能直接当作 RK3566 数据）。
- RK3566 规格表：`Video Encoding: 1080P@100fps H.265, 1080P@60fps H.264`。1080p30 在规格内。
- 同 SoC 的 Radxa Zero 3W 上，`mpi_enc_test` 已实跑 720p H.264，码流可被 `avdec_h264` 正常解码（High profile level 4，4:2:0 8-bit），说明 MPP 1.5.0 + `/dev/mpp_service` 的编码链路在 RK3566 上成立。
- `mpp_check_support_format(MPP_CTX_ENC, coding)` 可在运行时确认具体 codec；`mpi_enc_test` 在不支持时会直接报错。

**结论口径**：
- “RK3566 + MPP 1.5.0 支持 H.264/H.265 1080p30” = **VERIFIED（硬件规格 + MPP 能力 + 同 SoC 编码链路）**。
- “本板 1080p30 实测 ≥30 fps” = **UNVERIFIED，spike 必须做**。按 1080p60/100 的规格，预期 H.264 1080p30 有 2× 余量，H.265 余量更大。

### 3.2 最省事接法排序

按“在本板 Ubuntu 20.04 + 现有依赖下，从零到可录制 1080p30”的实际工作量排序：

| 排名 | 接法 | 工作量 | 适用条件 | 结论 |
|---|---|---|---|---|
| 1 | **GStreamer `mpph264enc` / `mpph265enc`** | 修权限/装预编译插件；Rust 侧 `gstreamer-rs` 或 `gst-launch` 子进程 | 插件二进制含编码器，或能装 Radxa `gstreamer1.0-rockchip1_1.14-4` | **首选**；零 C 编译、零拷贝 NV12、自带 mux/parse |
| 2 | **直接 MPP C API + Rust FFI** | 手写/bindgen 生成 ~30 个声明 + 编码循环，约 500–800 行 | 需要精确控制 PTS/关键帧/每帧参数，或插件不可用 | **首选备选**；运行时只依赖已有 `librockchip_mpp.so` |
| 3 | FFmpeg 8.1+ CLI/库 | 自建 FFmpeg（或静态包）+ 进程/管道 | 需要现成 mux/滤镜；可接受大依赖 | 可行但重；Ubuntu 20.04 自带 4.2 不行 |
| 4 | `libv4l-rkmpp` | 高 | Chromium VDA/VEA 魔改 | 拒绝 |
| 5 | Rust crates（yscv/rs-mpp） | 低表面、高返工 | 无 | 拒绝直接依赖 |

> 注意：上表第 1 名成立的前提是“插件里确实有编码器”。如果 `strings` 证明插件二进制不含编码器，则第 1 名需要先安装/构建插件；在这种情况下，**端到端最省事可能变成第 2 名**，因为 Rust 交叉编译不需要在板上装 C 工具链。

### 3.3 需要额外安装什么？`librockchip-mpp-dev` 够不够？License 风险？

**已经够用的部分：**
- `librockchip-mpp-dev` 提供：
  - `/usr/include/rockchip/*.h`（`rk_mpi.h`、`mpp_frame.h`、`mpp_buffer.h`、`mpp_packet.h`、`rk_venc_cfg.h` 等）
  - `rockchip_mpp.pc`（`pkg-config --cflags --libs rockchip_mpp`）
- `librockchip-mpp1` 提供 `librockchip_mpp.so` 与 `librockchip_mpp.so.*`（Debian install 文件同时包含两者），链接 `-lrockchip_mpp` 可用。
- 对“直接 MPP C API + Rust FFI”来说，**dev 包已经够编译**；运行时只需要 `librockchip-mpp1`。

**不够、需要额外装的部分：**

| 目的 | 额外包/文件 | 说明 |
|---|---|---|
| 跑官方 `mpi_enc_test` | `rockchip-mpp-demos`（依赖 `librockchip-vpu0`） | `mpi_enc_test`/`mpp_info_test` 在 `usr/bin/*`，**不在 dev 包** |
| GStreamer 编码 | `gstreamer1.0-rockchip1`（Radxa bullseye，1.14-4）或自建 `gstreamer-rockchip` | 还需要 `librga2`（插件链接 `librga.so.2`） |
| 自建 GStreamer 插件 | `meson ninja-build pkg-config libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev libdrm-dev`；可选 `librga-dev` | 板上无 gcc，必须在 PC 交叉编译或 arm64 容器构建 |
| 自建 FFmpeg 8.1+ | FFmpeg 源码 + `librockchip-mpp-dev` + `libdrm-dev`；`./configure --enable-rkmpp --enable-libdrm` | Ubuntu 20.04 的 FFmpeg 4.2 没有 `h264_rkmpp` 编码器 |
| Rust bindgen | 构建机上的 `libclang`（`bindgen` 依赖） | 只解析头文件，不需要板上 gcc |

**License 确认：**

| 组件 | License | 风险 |
|---|---|---|
| rockchip-linux/mpp | **Apache-2.0**（`debian/copyright`；文件头多为 `Apache-2.0 OR MIT`） | 与项目 MIT OR Apache-2.0 兼容；动态链接无传染 |
| gstreamer-rockchip 插件 | **LGPL-2.1** | 动态加载插件，不修改则不传染；修改插件源码需按 LGPL 提供 |
| FFmpeg 官方 | **LGPL-2.1+**（不加 `--enable-gpl`） | 可分发的动态/静态链接需满足 LGPL；`rkmppenc.c` 本身 LGPL |
| nyanmisaka/ffmpeg-rockchip | LGPLv3（README 要求 `--enable-version3`） | 比 LGPL-2.1 更严格；注意合规 |
| libv4l-rkmpp | LGPL-2.1 | 不采用 |
| yscv-video-mpp | Apache-2.0 | 许可没问题，但代码正确性有问题 |
| rs-mpp | “Other”（不明确） | 避免 |
| rigaya/rkmppenc | MIT | 工具用途，风险低 |

---

## 4. 推荐给 tspi-cam 的方案

### 4.1 选择

**录制编码后端：GStreamer `mpph264enc`，Rust 侧用 `gstreamer-rs`。**
**预览：独立于编码链路，继续走 V4L2/EGL/egui；不要为了预览旋转在编码管线里插 `videoconvert`/`videoflip`。**

理由：
1. 板上已有 GStreamer + MPP；`mpph264enc` 很可能只是被 `/dev/mpp_service` 权限挡住，修复成本最低。
2. GStreamer 直接处理 NV12、DMA-BUF、`h264parse`、`qtmux`/`mp4mux`、PTS，项目已有 `gstreamer-rs` 计划，代码量最小。
3. 1080p30 远低于 RK3566 的 1080p60 H.264 上限，性能余量足够。
4. 编码器输出 Annex-B，`h264parse ! qtmux` 即可得到带时间戳的 MP4，便于 Gyroflow 离线处理。

### 4.2 建议管线

录制（H.264，摄像头 NV12）：

```text
v4l2src device=/dev/video0 io-mode=4
  ! video/x-raw,format=NV12,width=1920,height=1080,framerate=30/1
  ! mpph264enc
      rc-mode=cbr
      bps=6000000
      gop=30
      header-mode=each-idr
      profile=high
      level=4.0
  ! h264parse
  ! qtmux
  ! filesink location=/userdata/rec/clip.mp4
```

关键属性：

| 属性 | 建议值 | 原因 |
|---|---|---|
| `rc-mode` | `cbr` | 录制/后续稳定化都希望码率稳定；VBR 也可，但 CBR 更可预测 |
| `bps` | 4–8 Mbps（1080p30） | 动作相机场景；H.265 可 3–5 Mbps |
| `gop` | 30（1 s） | 便于 seek/丢帧恢复；不要用默认“每秒一个 IDR”之外的大 GOP |
| `header-mode` | `each-idr` | 默认 `first-frame` 只在第一帧写 SPS/PPS；丢首帧或后加入的消费者无法解码 |
| `profile` / `level` | `high` / `4.0` | 1080p30 标准组合；兼容性好 |
| `rotation` | 先 `0`，需要录旋转画面再试 `90` | 编码器支持硬件旋转；但会改变输出宽高，MP4 旋转元数据不一定被 Gyroflow 正确识别。**建议录制传感器原始方向，预览/离线再旋转**，除非实测通过 |
| `max-pending` / `GST_MPP_ENC_MAX_PENDING` | `2` | 摄像头 capture pool 可能只有 8 个 buffer；编码器默认最多持有 16 个会饿死 V4L2。RK3566 实测需要限制 |

不要做的事：
- 不要在编码前加 `videoconvert`：摄像头已是 NV12，转一次就是一次全帧 CPU/GPU 拷贝。
- 不要在编码前加 `videoflip`：RK3566 上 `videoflip` 的输出格式可能触发 RGA 失败并回退软件转换，实测会把 30 fps 打到 7–8 fps 并触发降频。旋转交给 UI（预览）或编码器 `rotation`（录制）。
- 不要用 `v4l2h264enc`：RK3566 BSP 的 VPU 不是 V4L2 M2M，`/dev/video*` 里没有编码节点。

### 4.3 风险与缓解

| 风险 | 表现 | 缓解 |
|---|---|---|
| `/dev/mpp_service` 权限 | `gst-inspect` 只有解码器；`mpi_enc_test` 退出 0 但输出空文件 | udev `0660 root:video` + 用户/服务加入 `video`；用文件大小而非退出码验收 |
| `/dev/rga` 权限/格式 | 元素存在，管线启动后 `RGA_BLIT fail`/`Try to use uninit rgaCtx` | 同一条 udev 规则覆盖 `/dev/rga`；输入保持 NV12；避免 `videoflip` |
| 插件二进制不含编码器 | `strings` 无 `mpph264enc` | 安装 Radxa `gstreamer1.0-rockchip1_1.14-4`（bullseye/glibc 2.31）或自建 `gstreamer-rockchip` |
| 插件 ABI | 1.14 构建的插件在 1.16 上不注册 | 已有 1.14 插件在 1.26.2 正常注册的实测；1.16 风险低。仍以 `gst-inspect` 为准 |
| 缓冲区饥饿 | 摄像头 3 秒后停流、`bufcnt:0` | `GST_MPP_ENC_MAX_PENDING=2`；`mpph264enc max-pending=2` |
| 停止录制文件损坏 | MP4 无 moov | 停止时发送 EOS，等 `GST_MESSAGE_EOS` 后再置 NULL；不要直接 kill 进程 |
| H.265 兼容性 | 部分播放器/Gyroflow 不吃 | 默认 H.264；H.265 作为可选，先验证 Gyroflow/播放器 |
| MPP 1.5.0 旧 | 新参数/行为差异 | 以 1.5.0 头文件为准；不要照抄 develop 分支的新 `mpp_enc_cfg_create`；`mpp_enc_cfg_init` 在 1.5.0 可用 |

### 4.4 启动速度

- GStreamer 管线 `READY → PAUSED` 的预滚约几百毫秒；可在 daemon 启动时先把管线建到 `PAUSED`，录制时 `PLAYING`，停止时发 EOS 回 `PAUSED`。
- 录制开始时强制一个 IDR：向下游发 `GstVideoForceKeyUnit` 事件（`gst_video_event_new_downstream_force_key_unit`），确保第一帧可独立解码。
- 如果启动时间要求极严（<100 ms），直接 MPP FFI 更快；但当前“启动快”没有量化指标，GStreamer 足够。

### 4.5 与 IMU 同步

- 视频 PTS 用 `GstBuffer.pts`（`v4l2src` 从驱动取时间戳；必要时 `do-timestamp=true`）。
- IMU 采样用 `CLOCK_MONOTONIC`，录制开始时记录同一个时钟基准，写入 sidecar（如 `clip.imu`）。
- 不要在 GStreamer 之后再重新打时间戳；`mpph264enc` 会把输入 PTS 带到输出 packet。
- 如果使用 `mpi_enc_test` 或自写 FFI，务必 `mpp_frame_set_pts(frame, monotonic_us)`，否则 Gyroflow 无法对齐。

---

## 5. MPP 编码 API 关键调用序列

以下以 MPP 1.5.0 的 C API 为准（GStreamer 插件内部也是这套）。

### 5.1 初始化

```c
MppCtx ctx = NULL;
MppApi *mpi = NULL;
MppEncCfg cfg = NULL;

mpp_create(&ctx, &mpi);

// 可选：设置输出超时，避免 encode_get_packet 永久阻塞
RK_S64 timeout = 20000; // ms
mpi->control(ctx, MPP_SET_OUTPUT_TIMEOUT, &timeout);

// H.264: MPP_VIDEO_CodingAVC (7)
// H.265: MPP_VIDEO_CodingHEVC (0x01000004 = 16777220)
mpp_init(ctx, MPP_CTX_ENC, MPP_VIDEO_CodingAVC);

mpp_enc_cfg_init(&cfg);
mpi->control(ctx, MPP_ENC_GET_CFG, cfg);   // 取默认值，再覆盖
```

### 5.2 关键配置（`MppEncCfg` 字符串键）

| 键 | 值（1080p30 示例） | 说明 |
|---|---|---|
| `prep:width` | 1920 | 输入有效宽度 |
| `prep:height` | 1080 | 输入有效高度 |
| `prep:hor_stride` | 1920（16 对齐） | 行字节数；来自 V4L2 `bytesperline` 时必须用驱动值 |
| `prep:ver_stride` | 1080/1088（16 对齐） | 行数；NV12 的 `sizeimage` 可能含底部 padding |
| `prep:format` | `MPP_FMT_YUV420SP`（0，NV12） | 摄像头输出 NV12 时零转换 |
| `rc:mode` | `MPP_ENC_RC_MODE_CBR`（1） | 0=VBR，1=CBR，2=FIXQP，3=AVBR |
| `rc:bps_target` | 6000000 | CBR 目标码率 |
| `rc:fps_in_num` / `rc:fps_in_denom` | 30 / 1 | **注意拼写是 `denom`，不是 `denorm`** |
| `rc:fps_out_num` / `rc:fps_out_denom` | 30 / 1 | 输出帧率 |
| `rc:gop` | 30 | I 帧间隔 |
| `h264:profile` | 100（High）或 66（Baseline） | 录制用 High；需要最大兼容用 Baseline |
| `h264:level` | 40（4.0） | 1080p30 |
| `h264:cabac_en` | 1（High/Main） | Baseline 用 0 |
| `h265:profile` | 1（Main） | H.265 时 |
| `h265:level` | 120（4.0） | H.265 level 4.0 |

应用配置：

```c
mpi->control(ctx, MPP_ENC_SET_CFG, cfg);
mpp_enc_cfg_deinit(cfg);
```

可选：让每个 IDR 都重复 SPS/PPS（录制/网络更稳）：

```c
MppEncHeaderMode mode = MPP_ENC_HEADER_MODE_EACH_IDR; // 1
mpi->control(ctx, MPP_ENC_SET_HEADER_MODE, &mode);
```

### 5.3 取 SPS/PPS（必须）

```c
MppPacket hdr = NULL;
mpp_packet_init(&hdr, NULL, 0);           // 或 init_with_buffer 用预分配 buffer
mpi->control(ctx, MPP_ENC_GET_HDR_SYNC, hdr);

fwrite(mpp_packet_get_pos(hdr), 1, mpp_packet_get_length(hdr), out);
mpp_packet_deinit(&hdr);
```

时序：`MPP_ENC_SET_CFG` 之后、第一帧 `encode_put_frame` 之前。H.264 得到 SPS/PPS，H.265 得到 VPS/SPS/PPS。**不取 header，第一帧可能只有 slice，MP4/播放器无法解码。**

### 5.4 每帧循环

```c
while (running) {
    MppFrame frame = NULL;
    MppPacket packet = NULL;
    MppBuffer buf = NULL;

    // 1) 输入 buffer
    //    - 零拷贝：从 V4L2 dmabuf 导入
    MppBufferInfo info = {
        .type = MPP_BUFFER_TYPE_EXT_DMA, // 通用外部 dma_buf
        .fd   = v4l2_fd,
        .size = v4l2_sizeimage,
    };
    mpp_buffer_import(&buf, &info);

    //    - 或内部 DRM/DMA_HEAP 分配后 memcpy（非零拷贝，调试用）
    // mpp_buffer_group_get_internal(&grp, MPP_BUFFER_TYPE_DMA_HEAP, ...);
    // mpp_buffer_get(grp, &buf, hor_stride * ver_stride * 3 / 2);

    // 2) 帧描述
    mpp_frame_init(&frame);
    mpp_frame_set_width(frame, 1920);
    mpp_frame_set_height(frame, 1080);
    mpp_frame_set_hor_stride(frame, 1920);
    mpp_frame_set_ver_stride(frame, 1088);
    mpp_frame_set_fmt(frame, MPP_FMT_YUV420SP);
    mpp_frame_set_pts(frame, monotonic_us);
    mpp_frame_set_buffer(frame, buf);

    // 3) 输出 packet（可选：绑定预分配 buffer，减少拷贝）
    mpp_packet_init_with_buffer(&packet, pkt_buf);
    mpp_packet_set_length(packet, 0);
    mpp_meta_set_packet(mpp_frame_get_meta(frame),
                        KEY_OUTPUT_PACKET, packet);

    // 4) 送入编码器（阻塞直到硬件消费完输入）
    mpi->encode_put_frame(ctx, frame);
    mpp_frame_deinit(&frame);

    // 5) 取输出（可能一帧多个 slice packet）
    do {
        packet = NULL;
        mpi->encode_get_packet(ctx, &packet);
        if (!packet)
            break;
        fwrite(mpp_packet_get_pos(packet), 1,
               mpp_packet_get_length(packet), out);
        int eoi = mpp_packet_is_eoi(packet); // 分片时判断
        mpp_packet_deinit(&packet);
        if (eoi)
            break;
    } while (1);

    mpp_buffer_put(buf); // 导入的 buffer 释放引用，不关闭原 fd
}
```

结束：

```c
mpi->reset(ctx);
mpp_destroy(ctx);
```

### 5.5 常见坑

1. **`mpp_enc_cfg_init` 不是 `mpp_enc_cfg_create`**：MPP 1.5.0 用前者；`create` 是 2025+ 新接口。
2. **`hor_stride`/`ver_stride` 不能想当然等于宽高**：NV12 的 V4L2 `bytesperline` 可能带 padding；MPP 要求 16 对齐。用驱动返回的 `bytesperline`，`ver_stride` 用 `sizeimage / bytesperline`（或 `MPP_ALIGN(height,16)`）。
3. **`MPP_FMT_YUV420SP` = 0，`MPP_FMT_YUV420P` = 4**；不要照抄把 I420 写成 1 的绑定。
4. **`MPP_VIDEO_CodingHEVC` = 0x01000004**，不是 `0x10000001`。
5. **`rc:fps_in_denom`** 不是 `denorm`；拼错会被 MPP 忽略，码控按默认帧率算。
6. **必须取 `MPP_ENC_GET_HDR_SYNC`**；否则首帧无 SPS/PPS。
7. **`encode_put_frame` 是阻塞的**：它等硬件用完输入 buffer 才返回；不要在 UI 线程调用。
8. **输出是 Annex-B（`00 00 00 01` 起始码）**；写 MP4 前用 `h264parse`/`h265parse`，不要直接把裸流塞进 `qtmux`。
9. **`mpp_buffer_import` 的 fd 生命周期**：MPP 只持有引用，`mpp_buffer_put` 不会关闭原 fd；V4L2 buffer 要自己 qbuf。
10. **错误码不打印就查不到**：打开 `mpp_debug=1`、`mpi_debug=1` 环境变量；先跑 `mpp_info_test` 看版本/兼容层。

---

## 6. Spike 计划（板上命令级）

> 目标：用最小步骤证明“1080p30 H.264/H.265 编码可用”，再决定是否接 GStreamer 或 FFI。
> 所有命令在板上执行；板子无 gcc，不要试图在板上编译 C。

### Phase 0 — 盘点（2 分钟，不改系统）

```bash
uname -m; grep PRETTY /etc/os-release
ls -l /dev/mpp_service /dev/rga 2>&1
stat -c '%a %U:%G %n' /dev/mpp_service /dev/rga 2>&1
ldconfig -p | grep -E 'librockchip_mpp|librga'
dpkg -l | grep -E 'mpp|rockchip|gstreamer' | awk '{print $2, $3}'
gst-inspect-1.0 --version
gst-inspect-1.0 rockchipmpp 2>&1 | sed -n '1,60p'
gst-inspect-1.0 mpph264enc 2>&1 | head -20

# 关键：直接看插件二进制里有没有编码器
strings /usr/lib/aarch64-linux-gnu/gstreamer-1.0/libgstrockchipmpp.so 2>/dev/null \
  | grep -E '^mpph26[45]enc$|^mppjpegenc$|^mppvp8enc$'
```

判读：
- `strings` 有 `mpph264enc`/`mpph265enc`，但 `gst-inspect` 没有 → **权限问题**，进 Phase 1。
- `strings` 也没有 → 插件二进制确实不含编码器，进 Phase 1b/2。
- `/dev/mpp_service` 不存在 → 内核/BSP 问题，先确认是否 vendor kernel；本任务前提是存在。

### Phase 1 — 修 `/dev/mpp_service` 权限（临时验证）

```bash
sudo chmod 666 /dev/mpp_service
# 如果 /dev/rga 存在，也临时放开
sudo chmod 666 /dev/rga 2>/dev/null || true

# 清掉可能缓存的 registry，再查
rm -f ~/.cache/gstreamer-1.0/registry.*.bin
GST_REGISTRY_UPDATE=1 gst-inspect-1.0 mpph264enc | head -40
GST_REGISTRY_UPDATE=1 gst-inspect-1.0 mpph265enc | head -20
```

若出现元素，做永久规则：

```bash
sudo tee /etc/udev/rules.d/99-mpp.rules >/dev/null <<'EOF'
KERNEL=="mpp_service", GROUP="video", MODE="0660"
KERNEL=="rga", GROUP="video", MODE="0660"
EOF

sudo udevadm control --reload-rules
sudo udevadm trigger
sudo usermod -aG video "$USER"
# 退出重新登录，或 newgrp video
```

systemd 服务（actiond）需要：

```ini
SupplementaryGroups=video
```

验证非 root 可用：

```bash
newgrp video
gst-inspect-1.0 mpph264enc >/dev/null && echo "mpph264enc OK"
```

### Phase 1b — 若插件二进制不含编码器

优先装 Radxa bullseye 预编译包（RK3566 同源，glibc 2.31 与 Ubuntu 20.04 匹配）：

```bash
R=https://radxa-repo.github.io/bullseye/pool/main

# 先确认板上 MPP/RGA 已存在；缺哪个补哪个
curl -fLO $R/m/mpp/librockchip-mpp1_1.5.0-1_arm64.deb
curl -fLO $R/m/mpp/librockchip-vpu0_1.5.0-1_arm64.deb
curl -fLO $R/m/mpp/rockchip-mpp-demos_1.5.0-1_arm64.deb
curl -fLO $R/libr/librga/librga2_2.2.0-1_arm64.deb
curl -fLO $R/g/gstreamer1.0-rockchip/gstreamer1.0-rockchip1_1.14-4_arm64.deb

sudo dpkg -i librockchip-mpp1_1.5.0-1_arm64.deb \
             librockchip-vpu0_1.5.0-1_arm64.deb \
             rockchip-mpp-demos_1.5.0-1_arm64.deb \
             librga2_2.2.0-1_arm64.deb \
             gstreamer1.0-rockchip1_1.14-4_arm64.deb
```

注意：
- 如果板上已有同版本 `librockchip-mpp1`，不要盲目覆盖；先 `dpkg -l | grep mpp`。
- 这些是直接 deb 下载，`dpkg -i` 不会自动解析依赖；上面已把 MPP/RGA/插件成套列出。
- 如果 Radxa 包 URL 失效，备选是 `numbqq/gstreamer-rockchip-debs` 的 1.14-8；但它是 Ubuntu 22.04 构建，**要求 glibc >= 2.33**，Ubuntu 20.04 的 2.31 不兼容，不要直接用。
- 若预编译包都不可用，再考虑在 PC 上交叉编译 `gstreamer-rockchip`（不要板上编译）。

### Phase 2 — 官方 `mpi_enc_test` 验证 1080p30

先确认演示程序：

```bash
command -v mpi_enc_test || dpkg -L rockchip-mpp-demos | grep mpi_enc_test
```

H.264 1080p30，300 帧（10 秒素材），CBR 4 Mbps：

```bash
sudo mpi_enc_test -w 1920 -h 1080 -t 7 -n 300 -rc 1 -bps 4000000 -v f \
  -o /tmp/1080p30.h264
```

H.265 1080p30：

```bash
sudo mpi_enc_test -w 1920 -h 1080 -t 16777220 -n 300 -rc 1 -bps 3000000 -v f \
  -o /tmp/1080p30.h265
```

判读：
- 日志应出现 `chn 0 encode 300 frames time ... fps ...`；`fps` 必须 **≥30**，理想 ≥60（文件输入不受实时节流，代表吞吐上限）。
- **不要只看退出码**：`ls -l /tmp/1080p30.h26*`，文件必须远大于 0。权限错误时 `mpi_enc_test` 会写出空文件并退出 0。
- 如果 H.265 失败，先确认 `-t 16777220` 是否被 1.5.0 的 `mpi_enc_test` 接受；失败日志会给出 `mpp_check_support_format` 或 `mpp_init` 错误。

码流校验（板上 ffmpeg 4.2 即可）：

```bash
ffprobe -v error -show_streams -count_frames /tmp/1080p30.h264 \
  | grep -E 'codec_name|width|height|r_frame_rate|nb_read_frames|profile'
ffmpeg -v error -i /tmp/1080p30.h264 -f null - && echo "decode OK"
```

期望：`h264`、1920×1080、30/1、High profile、300 帧、解码无错误。

### Phase 3 — GStreamer 1080p30 实时管线（300 帧）

H.264：

```bash
/usr/bin/time -f 'wall=%e s cpu=%P maxrss=%M KB' \
gst-launch-1.0 -v \
  videotestsrc is-live=true num-buffers=300 \
  ! video/x-raw,format=NV12,width=1920,height=1080,framerate=30/1 \
  ! mpph264enc rc-mode=cbr bps=6000000 gop=30 header-mode=each-idr profile=high level=4.0 \
  ! h264parse \
  ! qtmux \
  ! filesink location=/tmp/gst-1080p30.mp4
```

吞吐测试（不按实时节流）：

```bash
gst-launch-1.0 -v \
  videotestsrc num-buffers=300 \
  ! video/x-raw,format=NV12,width=1920,height=1080,framerate=30/1 \
  ! mpph264enc rc-mode=cbr bps=6000000 gop=30 header-mode=each-idr \
  ! fpsdisplaysink video-sink=fakesink text-overlay=false
```

H.265 把 `mpph264enc` 换成 `mpph265enc`，`h264parse` 换成 `h265parse`。

判读：
- `wall` 对 300 帧 30 fps 应约 10–11 s（含启动）；明显更慢说明管线在软件转换或丢帧。
- 无 `ERROR`/`WARNING`；`ffprobe` 能读出 1920×1080/30 fps/300 帧。
- 如果 `mpph264enc` 报 “not-negotiated” 或 caps 错误，先确认输入确实是 NV12、宽高 16 对齐。

### Phase 4 — 摄像头 NV12 → 编码（可选，但这是项目真实路径）

```bash
v4l2-ctl --list-devices
v4l2-ctl -d /dev/video0 --list-formats-ext
```

找到摄像头支持的 NV12 分辨率和帧率后：

```bash
gst-launch-1.0 -v \
  v4l2src device=/dev/video0 io-mode=4 \
  ! video/x-raw,format=NV12,width=1920,height=1080,framerate=30/1 \
  ! mpph264enc rc-mode=cbr bps=6000000 gop=30 header-mode=each-idr \
  ! h264parse ! qtmux ! filesink location=/tmp/cam-1080p30.mp4
```

注意：
- `io-mode=4` 是 dmabuf 模式；如果驱动不支持，改 `io-mode=2`/`mmap`，但零拷贝会丢失。
- RK3566 的 rkisp 驱动给 `v4l2src` 的 buffer pool 可能很浅，实测会掉到 ~20 fps；如果这样，用 `v4l2-ctl --stream-mmap` + `fdsrc`/`appsrc` 喂 NV12，而不是硬用 `v4l2src`。
- OV5695 未必直接输出 1080p30 NV12；若只给 2592×1944，用 `mpph264enc width=1920 height=1080` 让编码器经 RGA 缩放，或先确认 sensor mode。

### Phase 5 — Rust 侧最小接入（二选一）

**5A（推荐先做）：`gstreamer-rs` 管线**

- 用 `gst::Pipeline::new()` 拼 `v4l2src → capsfilter(NV12/1920x1080/30) → mpph264enc → h264parse → qtmux → filesink`。
- 录制开始：`pipeline.set_state(Playing)`；向下游发 force-key-unit 事件。
- 录制停止：`send_event(Eos)`，在 bus 上等 `Eos`，再 `set_state(Null)`。
- 错误：监听 `Message::Error`，把 `mpph264enc` 的 GStreamer 错误映射到 daemon 状态。

**5B（备选）：薄 MPP FFI**

- 依赖：`libloading`（运行时 `dlopen("librockchip_mpp.so.1")`）+ `bindgen` 生成的类型/常量（构建机解析 `/usr/include/rockchip/*.h`）。
- 不要手写 `MppApi` 结构体；不要用 `yscv-video-mpp` 的常量（HEVC/YUV420P 值错误）。
- 最小验证：生成 300 帧 1920×1080 NV12（棋盘/渐变），走 §5 的 create/init/cfg/header/put/get 循环，统计 fps 和输出大小。
- 通过标准：≥30 fps、输出可被 `ffprobe` 识别、SPS/PPS 存在。

---

## 7. UNVERIFIED / 待确认清单

| 项 | 状态 | 如何确认 |
|---|---|---|
| 本板 RK3566 1080p30 H.264 实测 fps | `UNVERIFIED` | Phase 2 `mpi_enc_test` / Phase 3 GStreamer |
| 本板 RK3566 1080p30 H.265 实测 fps | `UNVERIFIED` | Phase 2 `-t 16777220` |
| 板上 GStreamer 精确版本（1.16.x?） | `INFERENCE` | `gst-inspect-1.0 --version` |
| `gstreamer1.0-rockchip1_1.14-4` 在 focal 的 glibc 依赖 | `UNVERIFIED` | `curl -fLO ...deb && dpkg-deb -I ... | grep Depends` |
| Radxa 预编译包 URL 当前是否可用 | `UNVERIFIED` | `curl -I` 各 .deb |
| `gst-rkmpp` 的 canonical 仓库/历史 | `UNVERIFIED`（公开 404） | 无需追；编码器在 `gstreamer-rockchip` |
| `nyanmisaka/ffmpeg-rockchip` 对 RK3566 的支持 | `UNVERIFIED` | 上游 README 只写 RK3588；需实测 |
| `rigaya/rkmppenc` 对 RK3566 的支持 | `UNVERIFIED` | README 未列 RK3566 |
| `rknn_cam_rtsp` 的 License | `UNVERIFIED` | 仓库未标；仅作代码参考，不复制 |
| `yscv-video-mpp` 在真实板上的可用性 | `UNVERIFIED`（源码常量已错） | 不建议投入；如要用，先修常量并补 header |
| MPP 1.5.0 的 `mpp_buffer_import` 对 V4L2 dmabuf 的 type 选择 | `INFERENCE` | 用 `MPP_BUFFER_TYPE_EXT_DMA` 先试；失败再试 `MPP_BUFFER_TYPE_DRM` |
| GStreamer 1.14 插件在 1.16 的 ABI | `INFERENCE`（有 1.14→1.26 实测） | Phase 1 `gst-inspect` |
| 编码器 `rotation=90` 后 Gyroflow 能否正确读取方向 | `UNVERIFIED` | 录一段旋转视频，用 Gyroflow 导入验证；否则录原始方向 |

---

## 8. 参考链接

**官方/一手**
- [rockchip-linux/mpp](https://github.com/rockchip-linux/mpp) — MPP 源码、LICENSES、debian/copyright
- [MPP 开发指南（EN）](https://github.com/rockchip-linux/mpp/blob/develop/doc/Rockchip_Developer_Guide_MPP_EN.md) — 编码流程、MppEncCfg 参数表、mpi_enc_test
- [mpi_enc_test.c](https://github.com/rockchip-linux/mpp/blob/develop/test/mpi_enc_test.c) — `MPP_ENC_GET_HDR_SYNC`、`encode_put_frame`、`encode_get_packet`
- [rk_mpi_cmd.h](https://github.com/rockchip-linux/mpp/blob/develop/inc/rk_mpi_cmd.h) — `MPP_ENC_SET_CFG`、`MPP_ENC_GET_HDR_SYNC`、`MPP_ENC_SET_HEADER_MODE`
- [rk_venc_cfg.h](https://github.com/rockchip-linux/mpp/blob/develop/inc/rk_venc_cfg.h) — `mpp_enc_cfg_init`
- [mpp_frame.h](https://github.com/rockchip-linux/mpp/blob/develop/inc/mpp_frame.h) — `MPP_FMT_YUV420SP=0`、`MPP_FMT_YUV420P=4`
- [rk_venc_rc.h](https://github.com/rockchip-linux/mpp/blob/develop/inc/rk_venc_rc.h) — RC 模式枚举
- [Firefly ROC-RK3566-PC 规格书](https://download.t-firefly.com/Spec/Mainboards/ROC-RK3566-PC_Specification_EN.pdf) — 1080p100 H.265 / 1080p60 H.264 编码

**GStreamer**
- [JeffyCN/mirrors@gstreamer-rockchip](https://github.com/JeffyCN/mirrors/tree/gstreamer-rockchip/gst/rockchipmpp) — `gstmpph264enc.c`、`gstmpph265enc.c`、`gstmppenc.c`
- [Fruit-Pi/gstreamer-rockchip](https://github.com/Fruit-Pi/gstreamer-rockchip) — 可浏览的完整源码镜像
- [Firefly MPP 硬件编码文档](https://community.t-firefly.com/en/docs/software/multimedia/GStreamer/gstreamer-rockchip-encoding) — `mpph264enc`/`mpph265enc` 管线和属性
- [Ubuntu Rockchip issue #773](https://github.com/Joshua-Riek/ubuntu-rockchip/issues/773) — `mpph26xenc` 卡顿回归及 2024-05 修复
- [microduck media-bringup](https://github.com/pollen-robotics/microduck/blob/main/docs/project/media-bringup.md) — RK3566 上 `/dev/mpp_service` 权限陷阱、`strings` 验证、`mpph264enc` 实测
- [microduck setup-gstreamer.sh](https://github.com/pollen-robotics/microduck/blob/main/scripts/setup-gstreamer.sh) — udev 规则、MPP/RGA deb 安装、编码器探测

**FFmpeg**
- [FFmpeg rkmppenc.c](https://ffmpeg.org/doxygen/trunk/rkmppenc_8c_source.html) — 官方编码器实现
- [FFmpeg Changelog](https://github.com/FFmpeg/FFmpeg/blob/master/Changelog) — 8.1 “Rockchip H.264/HEVC hardware encoder”
- [nyanmisaka/ffmpeg-rockchip](https://github.com/nyanmisaka/ffmpeg-rockchip) — `h264_rkmpp`/`hevc_rkmpp`，目标 RK3588

**其他**
- [JeffyCN/libv4l-rkmpp](https://github.com/JeffyCN/libv4l-rkmpp) — VDA/VEA、`libv4l-rkmpp-enc.c`
- [yscv-video-mpp](https://crates.io/crates/yscv-video-mpp) — Rust 封装（仅作反面参考）
- [rainbowflesh/rs-mpp](https://github.com/rainbowflesh/rs-mpp) — 仅 `mpp_log`
- [shum-elli/rknn_cam_rtsp](https://github.com/shum-elli/rknn_cam_rtsp) — RK3566 上 MPP H.264 + GetHeader 参考
- [rigaya/rkmppenc](https://github.com/rigaya/rkmppenc) — MPP 编码性能/质量工具
- [Radxa bullseye pool](https://radxa-repo.github.io/bullseye/pool/main) — MPP/RGA/`gstreamer1.0-rockchip1` deb 来源