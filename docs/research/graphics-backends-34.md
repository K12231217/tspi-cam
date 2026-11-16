# RK3566（Mali-G52 + Rockchip libmali 1.9）Rust 图形后端调研

- 调研日期：2026-11-16
- 目标板：泰山派 1M / RK3566 / Ubuntu 20.04.6 aarch64 / 480×800@60 HDMI + USB 电容触摸
- 约束：无 Wayland 合成器、无窗口管理器、板上无 gcc；只读调研，全部结论需有文件/源码/日志依据
- 标记：`[V]` = 已在板上文件/日志或上游源码中核实；`[U]` = 未核实/仅有文档；`[I]` = 由已核实事实推导

---

## 0. 结论（先看这里）

1. **本项目当前选 Path A：X11 + libmali(EGL) + egui/eframe + wgpu(GL/GLES)。**
   原因：egui 已经在项目计划里；Xorg 已装、触摸已能被 libinput 识别；`libmali` 的 EGL 在 Xorg glamor 下已在 Mali-G52 上初始化成功 `[V]`；不需要装新包。**必须走 EGL，不能走 GLX**——Xorg 日志明确显示 GLX 回退到 `swrast` `[V]`。

2. **(a) X11 变体 libmali 能否在无 X 时用 GBM 平台初始化？**
   **强迹象为“能”，但本报告不把它标成已证实。** Xorg 1.20.13 的 glamor 是通过 `gbm_create_device()` + `EGL_PLATFORM_GBM_MESA(0x31D7)` 初始化的，日志显示 `glamor X acceleration enabled on Mali-G52` `[V]`；`/etc/ld.so.conf.d/00-aarch64-mali.conf` 把 mali 目录放在最前，`mali.pc`/包内也带 `libgbm.so.1` `[V]`。但无法仅凭日志排除“glamor 实际用了 Rockchip 定制 Mesa 的 EGL”这一可能，所以 **GBM 无 X 直连标 `UNVERIFIED（强迹象）`**，用第 5 节的 ctypes 探针 10 分钟可定论。

3. **(b) 纯 DRM/KMS + dumb buffer 软绘制能否 480×800@30–60？**
   **架构上可行，板上未实测。** 480×800×4B×30 = 46 MB/s，×60 = 92 MB/s，对 LPDDR 是零头；`drm-rs::DumbBuffer`、LVGL DRM dumb-buffer 后端、Slint `linuxkms-software`、GStreamer `kmssink` 都走这条路 `[V]`。已知坑是 **dumb buffer 读取极慢、写入正常**，所以只能“只写不读” `[V]`。实际 fps 必须用第 5 节的 `kmssink` 命令实测 `[U]`。

4. **(c) 是否必须启动 Xorg 才能 GPU 加速？**
   **不是必须，但如果用 egui/eframe 就是必须的**（egui/winit 没有 KMS 后端）`[V]`。Xorg 的代价：本次日志中的 Xorg 初始化约 **3.7 s** `[V]`；内存未测 `[U]`（经验值 30–60 MB `[I]`）；按 `20-modesetting.conf` 的 `FlipFB=always` 注释，吞吐可能损失最多 50% `[V]`；且 GLX 仍是软渲染 `[V]`。

**一句话：M2 用 A 交付；如果 M3 的启动时间/内存预算不过，再迁 B（GBM/EGL，保留 egui）或 C（KMS 软绘制 + Slint/tiny-skia），不要把 Wayland 纳入方案。**

---

## 1. 板上基线（本次实际读到的证据）

| 项 | 值 | 证据 |
|---|---|---|
| OS / 内核 | Ubuntu 20.04.6 LTS；Linux 4.19.232 aarch64 | `ssh://tspi/etc/os-release`；`/var/log/Xorg.0.log` 头 |
| Xorg | X.Org 1.20.13，`modesetting`，`glamor`，`DRI driver: rockchip` | `/var/log/Xorg.0.log`（`glamor X acceleration enabled on Mali-G52`） |
| GLX | **软渲染**：`AIGLX error: Calling driver entry point failed` → `GLX: Initialized DRISWRAST GL provider` | `/var/log/Xorg.0.log` |
| 面板模式 | `480x800`@60.0，HDMI-1 | `/var/log/Xorg.0.log` modeline |
| 触摸 | `WaveShare WS170120` 被 libinput 识别为 touchscreen | `/var/log/Xorg.0.log` |
| libmali | `libmali.so.1.9.0`、`libmali-bifrost-g52-g2p0-x11.so`；包名 `libmali-bifrost-g52-g2p0-x11` | `/var/lib/dpkg/info/libmali-bifrost-g52-g2p0-x11.list` |
| mali 附带库 | `mali/libEGL.so.1`、`mali/libGLESv2.so.2`、`mali/libgbm.so.1`、`mali/libMaliOpenCL.so.1` | 同上 + `/usr/lib/aarch64-linux-gnu/mali/` 目录 |
| loader 顺序 | `/etc/ld.so.conf.d/00-aarch64-mali.conf` 内容为 `/usr/lib/aarch64-linux-gnu/mali` | 直接读取 |
| `mali.pc` | `Version: 1.9.0`；`Requires: libdrm, x11, xcb, x11-xcb, xcb-dri2` | `/usr/lib/aarch64-linux-gnu/pkgconfig/mali.pc` |
| GStreamer 插件 | `libgstkms.so`、`libgstrockchipmpp.so`、`libgstrkximage.so` 都在 | `/usr/lib/aarch64-linux-gnu/gstreamer-1.0/` |
| 工具 | `gst-launch-1.0`、`gst-inspect-1.0`、`python3`、`Xorg` 存在；`modetest`、`kmscube` **不存在** | 逐路径检查 |
| 输入/字体库 | `libinput.so.10`、`libudev.so.1`、`libxkbcommon.so.0`、`libfontconfig.so.1`、DejaVu 字体存在；`libseat.so.1` **不存在** | 逐路径检查 |
| DRI | `rockchip_dri.so`、`kms_swrast_dri.so`、`swrast_dri.so` | `/usr/lib/aarch64-linux-gnu/dri/` |

---

## 2. 34 个仓库结论表

> ★ 为 2026-11-16 通过 shields.io / GitLab API 读取的约数；GitLab 仓库单独标注。  
> “板上可运行”一律为 `UNVERIFIED`（除明确写了日志/文件证据的），本表判定的是“是否值得进入技术栈/作为参考”。

| # | 仓库 | ★ | License | 最近 release | 关键文件 / 证据 | 结论 |
|---:|---|---:|---|---|---|---|
| 1 | [tsukumijima/libmali-rockchip](https://github.com/tsukumijima/libmali-rockchip) | 251 | ARM EULA（blob，`CLOSED`）；Debian 脚本 GPL-2+ | `v1.9-1-20260923-ec78b74` | [`debian/targets`](https://github.com/tsukumijima/libmali-rockchip/blob/master/debian/targets) 按 SoC 列 `-gbm`/`-x11-gbm`；release 资产含 `libmali-bifrost-g52-g2p0-gbm` 13.5 MB；`debian/control.in` 依赖 `libgbm-dev/libdrm-dev/libx11-xcb1` | **采用**（blob/包名基准；板上已装 x11 包） |
| 2 | [armbian/libmali-rockchip](https://github.com/armbian/libmali-rockchip) | 7 | 同上（fork） | `v1.9-1-20260304-9b413d2` | fork 自 #1，release 资产同样分 `-gbm`/`-x11-gbm` | **备选**（离线 .deb 来源） |
| 3 | [JeffyCN/mirrors](https://github.com/JeffyCN/mirrors/tree/libmali) | 114 | 混合/无顶层 LICENSE | 无 release（分支 `libmali`） | 上游 `libmali` 镜像；`include/GBM/`、`include/EGL/`；ROCKNIX README 明确其为镜像源 | **备选**（查上游构建选项/WSI 组合） |
| 4 | [armbian/build](https://github.com/armbian/build) | 5.5k | GPL-2.0 | `v26.11.0-trunk.125` | 板级配置、内核/固件/驱动打包脚本（`config/boards/`、`packages/bsp/` 目录） | **备选**（镜像/打包参考，不是图形 API） |
| 5 | [Joshua-Riek/ubuntu-rockchip](https://github.com/Joshua-Riek/ubuntu-rockchip) | 4.1k | GPL-3.0 | `v2.4.0` | README：`3D hardware acceleration via panfork`、GNOME/Wayland；面向 22.04/24.04 + 5.10/6.1，不是本板 4.19 BSP | **排除**（panfork+Wayland 路线与本板内核/镜像不匹配） |
| 6 | [rockchip-linux/kernel](https://github.com/rockchip-linux/kernel) | 1482 | GPL-2.0 | 无 release（`develop-6.1`） | BSP 内核；本板实际使用 `drivers/gpu/arm/mali400` + `/dev/mali0` | **排除**（仅用于确认 mali400/DRM/VOP2 能力，不编译、不依赖） |
| 7 | [airockchip/librga](https://github.com/airockchip/librga) | 603 | Apache-2.0 | `v1.10.0` | README 明确支持 **RK3566**；`include/im2d.h` 提供 rotate/scale/format convert；预编译 `libs/Linux/gcc-aarch64/librga.so` | **采用**（预览 90° 旋转 + letterbox/缩放的硬件候选） |
| 8 | [rockchip-linux/mpp](https://github.com/rockchip-linux/mpp) | 1112 | Apache-2.0 + MIT（`LICENSES/`） | tag `v1.1.0` | README：支持 RK3566/RK3568；`mpp/codec/dec/jpeg`、`mpp/codec/enc/jpeg`；板上 `libgstrockchipmpp.so` 已装 | **备选**（硬解 MJPEG / 硬编，显著降 CPU） |
| 9 | [dvdhrm/docs](https://github.com/dvdhrm/docs/tree/master/drm-howto) | 506 | MIT（文件头；仓库无 LICENSE） | 无（最后 push 2022-05） | `drm-howto/modeset-double-buffered.c`、`modeset-vsync.c`、`modeset-atomic.c`；经典 dumb buffer + KMS 教学 | **备选**（Path C 的直接代码模板） |
| 10 | [mesa/kmscube](https://gitlab.freedesktop.org/mesa/kmscube) | 31（GitLab） | MIT | 无 release（最后活动 2026-05） | `kmscube.c` → `init_gbm()`/`init_egl()`；`common.c` 用 `eglGetPlatformDisplayEXT(EGL_PLATFORM_GBM_KHR, gbm->dev, NULL)`，失败回退 `eglGetDisplay(gbm)`；`drm-common.c` 含 legacy/atomic 两套 | **采用（模板）**（Path B 的标准参考实现） |
| 11 | [libsdl-org/SDL](https://github.com/libsdl-org/SDL/tree/main/src/video/kmsdrm) | 17k | Zlib | `release-3.4.20`（SDL2 另线） | `src/video/kmsdrm/SDL_kmsdrmvideo.c`、`SDL_kmsdrmopengles.c`；KMSDRM 后端直接 DRM+GBM/EGL，需 DRM master | **备选**（板上 SDL2 是否带 kmsdrm 未核实 `[U]`） |
| 12 | [mesa/mesa](https://gitlab.freedesktop.org/mesa/mesa) | 682（GitLab） | MIT | `mesa-26.2.7` | Panfrost 文档：支持 G52，但需要 panfrost 内核驱动；本板内核是 mali400 | **排除**（当前内核无 panfrost；不能用 panfrost 替代 blob） |
| 13 | [mesa/libdrm](https://gitlab.freedesktop.org/mesa/libdrm) | 78（GitLab） | MIT | `libdrm-2.4.134` | `tests/modetest/buffers.c`：`drmModeCreateDumbBuffer`/`drmModeMapDumbBuffer`；`tests/modetest/modetest.c` | **采用**（系统库 + dumb buffer API/工具参考；板上未装 modetest） |
| 14 | [KhronosGroup/EGL-Registry](https://github.com/KhronosGroup/EGL-Registry) | 154 | Khronos 规格许可（仓库无 OSI LICENSE） | 无 release | `extensions/KHR/EGL_KHR_platform_gbm.txt`、`EGL_EXT_platform_base.txt`；`EGL_PLATFORM_GBM_KHR=0x31D7` | **备选**（规范查阅；不进入依赖） |
| 15 | [GStreamer/gstreamer](https://github.com/GStreamer/gstreamer) | 3.4k | LGPL-2.1+ | tag `v1.28.9` | `sys/kms/gstkmssink.c`、`gstkmsallocator.c`（dumb buffer 分配/`drm_mode_destroy_dumb`）；kmssink 支持 `plane-properties`（含 rotation）与 `can-scale`；板上 `libgstkms.so` 已装 | **采用**（Path C 的零新包预览/验证通道） |
| 16 | [mpv-player/mpv](https://github.com/mpv-player/mpv) | 38k | GPL-2.0+ | `v0.42.0` | `--gpu-context=drm`（GBM/EGL）+ `--vo=gpu`，无 X 运行；是“libmali/GBM 无 X 可用”的产品级旁证 | **备选**（验证参考，不集成） |
| 17 | [GStreamer/gstreamer-rs](https://github.com/GStreamer/gstreamer-rs) | 59 | MIT | `0.25.4` | `gstreamer/`、`gstreamer-app/`；appsink 取帧、`jpegdec`/`videoconvert` | **备选**（若 UI 自己解 MJPEG，用它能少写 C） |
| 18 | [smithay/drm-rs](https://github.com/smithay/drm-rs) | 115 | MIT | `0.15.0` | `src/control/dumbbuffer.rs`：`DumbBuffer::create_from_device()` / `map()`，注释明确 “slow, but generic”；atomic API | **采用**（Path B/C 的 KMS 基础） |
| 19 | [Smithay/gbm.rs](https://github.com/Smithay/gbm.rs) | 36 | MIT | `0.18.0` | README：`Device::new`、`create_buffer_object(SCANOUT\|WRITE)`、`set_crtc`；`drm-support` feature | **采用**（Path B 的 GBM 缓冲） |
| 20 | [timothee-haudebourg/khronos-egl](https://github.com/timothee-haudebourg/khronos-egl) | 27 | MIT/Apache-2.0 | `6.0.0` | 支持 `dynamic` 加载 `libEGL.so.1`；`get_platform_display`/`get_display`；README 示例 | **采用**（Path A/B 的 EGL 绑定） |
| 21 | [Nercury/egli-rs](https://github.com/Nercury/egli-rs) | 20 | Apache-2.0 | `0.5.0` | 轻量 EGL 动态加载接口（`egli`） | **备选**（比 khronos-egl 更薄，功能少） |
| 22 | [smithay/smithay](https://github.com/smithay/smithay) | 3.4k | MIT | `0.7.0` | Wayland compositor 库；`src/backend/drm/` 用 `libgbm`+`libEGL`；README 依赖含 `libgbm`/`libEGL` | **排除**（它是合成器框架，本项目不需要 Wayland 合成器） |
| 23 | [smithay/wayland-rs](https://github.com/smithay/wayland-rs) | 1.5k | MIT | `0.31.15` | Wayland client 协议绑定 | **排除**（板上无 Wayland 合成器） |
| 24 | [rust-windowing/glutin](https://github.com/rust-windowing/glutin) | 2.1k | Apache-2.0 | `0.32.3` | `DisplayApiPreference::Egl` / `EglThenGlx` / `GlxThenEgl`；可强制 EGL；但只接受窗口/Display 句柄，没有 KMS surface | **备选**（Path B 自绘时的 EGL 上下文；避免默认 GLX） |
| 25 | [rust-windowing/winit](https://github.com/rust-windowing/winit) | 6.3k | Apache-2.0 | `0.31.0-beta.4`（稳定线 0.30.x） | X11/Wayland 平台实现；X11 触摸走 XInput2；无 KMS 后端 | **采用**（Path A 的窗口/触摸） |
| 26 | [rust-windowing/softbuffer](https://github.com/rust-windowing/softbuffer) | 514 | Apache-2.0 | `0.4.8` | 仅 X11/Wayland surface 呈现；无 DRM/KMS 后端 | **排除**（对无 X 场景不可用） |
| 27 | [Rust-SDL2/rust-sdl2](https://github.com/Rust-SDL2/rust-sdl2) | 3k | MIT | `0.38.0` | SDL2 绑定；可用 `SDL_VIDEODRIVER=kmsdrm`（取决于板上 libSDL2 编译选项） | **备选**（比 egui 更低层；KMSDRM 版本未知 `[U]`） |
| 28 | [emilk/egui](https://github.com/emilk/egui) | 32k | MIT/Apache-2.0 | `0.37.0` | `crates/eframe/README.md`：native 用 `egui-winit`，渲染器 `wgpu`/`glow`；没有 KMS/DRM 后端 | **采用**（Path A 的 UI） |
| 29 | [gfx-rs/wgpu](https://github.com/gfx-rs/wgpu) | 19k | MIT/Apache-2.0 | `30.0.2` | `wgpu-hal/src/gles/egl.rs`：`EGL_PLATFORM_X11_KHR`、`EGL_PLATFORM_WAYLAND_KHR`、`EGL_MESA_platform_surfaceless`；无 KMS swapchain | **采用**（Path A 用 GL/GLES 后端走 EGL） |
| 30 | [linebender/tiny-skia](https://github.com/linebender/tiny-skia) | 1.7k | BSD-3-Clause | `0.12.0` | 纯 CPU 2D 光栅；无平台依赖；适合 Path C 自绘 | **备选**（Path C 的 UI/预览合成） |
| 31 | [emoon/rust_minifb](https://github.com/emoon/rust_minifb) | 1.2k | MIT | `0.29.0` | 跨平台窗口/帧缓冲；Linux 只走 X11/Wayland | **排除**（无 KMS） |
| 32 | [femtovg/femtovg](https://github.com/femtovg/femtovg) | 958 | MIT/Apache-2.0 | `0.28.0` | OpenGL ES 矢量画布；EGL 上下文需调用方提供（可配 GBM surface） | **备选**（Path B 自绘/或 Slint linuxkms-femtovg 的底层） |
| 33 | [slint-ui/slint](https://github.com/slint-ui/slint) | 25k | GPL-3.0-only / Royalty-free-2.0 / 商业 | `1.18.1` | `internal/backends/linuxkms/`：**无 X/Wayland**，DRM/KMS；`renderer-software` 走 DRM dumb buffers；`renderer-femtovg` 走 GBM+EGL；libinput 可选、libseat 可选；`SLINT_BACKEND=linuxkms-software`、`SLINT_KMS_ROTATION` | **备选**（唯一“开箱即用”的无 X Rust UI；但许可证/换框架成本） |
| 34 | [parasyte/pixels](https://github.com/parasyte/pixels) | 2.2k | MIT | `0.17.2` | 基于 wgpu 的像素 surface；仍需要窗口 surface | **排除**（无 KMS surface） |

---

## 3. 三条可用路径对比

### Path A：X11 + libmali（EGL/GLES）+ egui/eframe

**链路**：`eframe` → `winit`（X11）→ `wgpu`（GL/GLES）→ `khronos-egl` → `libmali` EGL/GLES → `/dev/mali0` → KMS/HDMI。

| 断言 | 状态 | 依据 |
|---|---|---|
| 板上有 Xorg 1.20.13 + modesetting + glamor | **VERIFIED** | `/var/log/Xorg.0.log`：`modeset(0): glamor X acceleration enabled on Mali-G52` |
| X11 的 GLX 不是 GPU | **VERIFIED** | 同日志：`AIGLX error: Calling driver entry point failed` → `GLX: Initialized DRISWRAST GL provider` |
| libmali 的 EGL 能在 X11 下用 Mali | **VERIFIED（间接）** | glamor 在 Mali-G52 上启用；`/etc/ld.so.conf.d/00-aarch64-mali.conf` 优先 mali 目录 |
| egui 原生后端只支持 X11/Wayland | **VERIFIED** | `eframe/README.md`：native 用 `egui-winit`；无 KMS 后端 |
| wgpu GL 后端在 Linux 走 EGL | **VERIFIED** | `wgpu-hal/src/gles/egl.rs` 使用 `EGL_PLATFORM_X11_KHR` 等 |
| 板上 egui 实际 fps / 内存 / 启动时间 | **UNVERIFIED** | 未运行 |

**优点**：已有 egui 计划；触摸/字体/输入现成；无需新包；Mali GPU 可参与纹理上传与合成；`wgpu` 可用 GLES 3.x。
**风险**：GLX 默认是 `swrast`，必须强制 EGL；libmali 是否暴露 `EGL_EXT_platform_x11` 未知；无 WM 时全屏/焦点需手动处理；`FlipFB=always` 可能降低吞吐；Xorg 常驻内存/启动成本。

**Xorg 代价（实测/推断）**：日志首行到初始化完成约 **3.7 s** `[V]`；RSS 未测 `[U]`；`FlipFB=always` 注释写明可能最多 50% 性能损失 `[V]`。

---

### Path B：无 X，GBM + EGL(libmali) + 自绘/egui_glow

**链路**：`drm`（打开 `/dev/dri/card0`，设 connector/crtc/mode）→ `gbm`（`gbm_create_device`、`gbm_surface_create(SCANOUT\|RENDERING)`）→ `khronos-egl`（`eglGetPlatformDisplayEXT(EGL_PLATFORM_GBM_KHR, gbm_dev, NULL)`，失败回退 `eglGetDisplay(gbm)`）→ GLES 渲染 → `gbm_surface_lock_front_buffer` → `drmModeAddFB2` → `drmModePageFlip`。

| 断言 | 状态 | 依据 |
|---|---|---|
| kmscube 的标准 GBM+EGL 初始化路径 | **VERIFIED** | `kmscube/common.c`：`gbm_create_device`、`eglGetPlatformDisplayEXT(EGL_PLATFORM_GBM_KHR, ...)`、`eglCreateWindowSurface(gbm_surface)` |
| 板上 libmali 支持 GBM EGL（至少在 Xorg 进程内） | **VERIFIED（进程内）** | Xorg glamor 用 `gbm_create_device` + `EGL_PLATFORM_GBM_MESA`，且渲染器是 Mali-G52 |
| 完全无 X 时同一 blob 仍能 GBM init | **UNVERIFIED（强迹象）** | 缺少直接 kmscube/ctypes 运行证据；需第 5 节探针 |
| 现有 Rust 生态能拼出该路径 | **VERIFIED** | `drm 0.15` + `gbm 0.18` + `khronos-egl 6.0`；Slint linuxkms-femtovg 用的就是 `drm`+`gbm`+`glutin(egl)` |
| egui 能直接跑在 KMS 上 | **VERIFIED（否）** | eframe 仅 winit；需自己用 `egui_glow` 画到 EGL surface |
| 工作量 | **INFERENCE** | 约 400–600 行胶水：DRM 选屏/模式、GBM surface、EGL、page flip、evdev/libinput、egui 输入映射 |

**优点**：无 Xorg，启动快、内存省；GPU 仍可用；可保留 egui（MIT/Apache）；不需要新包（若 GBM 探针通过）。
**风险**：需要自己处理 DRM master、page flip、双缓冲、触摸坐标、窗口失去焦点；libmali 的 GBM/GLES 扩展是否满足 wgpu/egui_glow 未知；调试成本明显高于 A。

---

### Path C：无 X，纯 KMS dumb buffer 软绘制（tiny-skia / egui 自绘 / Slint software）

**链路**：`drm` `DumbBuffer`（XRGB8888）→ CPU 光栅（tiny-skia / Slint software / 自绘）→ `mmap` 写入 → `drmModeAddFB` → page flip（双缓冲/脏矩形）。

| 断言 | 状态 | 依据 |
|---|---|---|
| dumb buffer 创建/映射/呈现可行 | **VERIFIED** | `drm-rs/src/control/dumbbuffer.rs`；`libdrm/tests/modetest/buffers.c`；`gstkmsallocator.c` 使用 `drm_mode_destroy_dumb` |
| 有完整 UI 框架可直接用 dumb buffer | **VERIFIED** | Slint `linuxkms-software` 文档：软件渲染走 DRM dumb buffers；无需任何 C 库 |
| dumb buffer 只写不读时性能可接受 | **VERIFIED（定性）** | eighty-twenty 实测：dumb buffer **读**极慢（约 16 MB/s 量级），**写**很快；内核 `DRM_CAP_DUMB_PREFER_SHADOW` 明确建议“streaming ordered memory copies… never read” |
| 480×800@30/60 的扫描带宽 | **VERIFIED（算术）** | 480×800×4×30 = 46.08 MB/s；×60 = 92.16 MB/s |
| 板上 30 fps 实际可达 | **UNVERIFIED** | 无本板实测；受 CPU 光栅、JPEG 解码、旋转/缩放影响 |
| 触摸输入 | **VERIFIED（方案）** | `libinput`/`evdev` 均可用；Slint linuxkms 可选 libinput；板上 `libinput.so.10` 存在 |

**优点**：零 X、零 GPU、依赖最少（libdrm + 字体）；启动最快；480×800 的像素量很小；Slint software 还支持局部刷新。
**风险**：预览的 MJPEG 解码 + 90° 旋转 + letterbox 全在 CPU，30 fps 可能吃紧；dumb buffer 不能读回；页面翻转/脏矩形要自己写；Slint 有 GPL/royalty-free 许可证问题。

---

## 4. 三个问题的直接回答

### (a) X11 变体 libmali 在无 X 时能否用 GBM 平台初始化？

- **已核实的事实**：该包不是“纯 X11 WSI”那么简单——它自带 `mali/libgbm.so.1`，且 Xorg 的 glamor 走了 `gbm_create_device()` + `EGL_PLATFORM_GBM_MESA`，并在 Mali-G52 上初始化成功。
- **结论**：**很可能可以**；`EGL_PLATFORM_GBM_KHR`（数值同 `EGL_PLATFORM_GBM_MESA`，0x31D7）或回退 `eglGetDisplay(gbm)` 都有成功先例（Xorg 进程内）。但“无 X 独立进程”仍标 **UNVERIFIED**，因为 Xorg 进程内可能仍依赖 X 连接做 EGL 初始化。
- **定论方法**：第 5 节 ctypes 探针；若 `eglInitialize` 返回 1 且 vendor 含 `ARM`/`Mali`，即通过。失败则需 `libmali-bifrost-g52-g2p0-gbm`（13.5 MB，需新包）或退回 A。

### (b) 纯 DRM/KMS + dumb buffer 软绘制能否 480×800@30–60？

- **能到 30–60 的瓶颈不在扫描输出**：46/92 MB/s 对 LPDDR 很低；`kmssink` 和 Slint 都已在生产嵌入式场景使用 dumb buffer。
- **瓶颈在 CPU**：每帧 1.536 MB 的绘制/拷贝、JPEG 解码、90° 旋转 + letterbox。若 UI 只重绘按钮/文字（脏矩形），30 fps 很现实；若每帧全屏重绘 + 解码 + 旋转，Cortex-A55 上可能掉到 15–20 fps。
- **已有实测数据**：本板没有；上游只有定性结论（dumb buffer 写入快、读取慢）。**标 UNVERIFIED**，用 `kmssink` 命令实测。

### (c) 是否必须启动 Xorg 才能 GPU 加速？代价是什么？

- **不是必须**：GPU 走的是 EGL/GLES，GBM 平台可以在无 X 下用；kmscube/mpv 都证明了这一点。**但 egui/eframe 必须**，因为它只有 winit(X11/Wayland) 后端。
- **Xorg 代价**：
  - 启动：本次日志约 **3.7 s**（首行到 glamor/输入初始化完成）`[V]`；
  - 内存：未测 `[U]`，经验值 30–60 MB `[I]`；
  - 性能：`20-modesetting.conf` 的 `FlipFB=always` 注释写明可能最多 50% 损失 `[V]`；
  - 功能：GLX 仍是 `swrast`，任何走 GLX 的 toolkit 都拿不到 Mali；只有 EGL 路径有 GPU `[V]`；
  - 运维：需要 display-manager/手动 Xorg、无 WM 时窗口全屏/焦点要自己处理。

---

## 5. 推荐路径与最小验证

### 5.1 推荐

**现在选 Path A：Xorg（无 WM，kiosk）+ libmali EGL + `eframe`/`egui` + `wgpu` GL/GLES 渲染器。**

配置要点：

- `eframe` 使用 `wgpu` 渲染器（不要用默认可能走 GLX 的 `glow`）；启用 `x11`，禁用 `wayland`。
- 运行前确保 `LD_LIBRARY_PATH`/`ldconfig` 让 `libEGL.so.1` 解析到 `/usr/lib/aarch64-linux-gnu/mali/`（板上已由 `/etc/ld.so.conf.d/00-aarch64-mali.conf` 保证）。
- 启动窗口用 `with_inner_size([480,800])` + `with_position([0,0])` + `with_decorations(false)`；无 WM 时不要依赖 `_NET_WM_STATE_FULLSCREEN`。
- 预览：把解码后的 RGBA 上传为 `egui::TextureHandle`；90° 旋转用 `egui::Mesh` 改 UV（避免每帧 CPU 旋转），letterbox 用目标矩形；若 CPU 解码/旋转吃紧，接 `librga` 做 NV12 旋转/缩放。
- 触摸：winit 的 Touch 事件，把 `0..481 / 0..801` 线性映射到 `480×800`；校准偏移写配置。

**为什么不是 B/C（当前）**：A 是唯一“已有 egui + 已装 Xorg + 已证明 EGL 可用”的组合，M2 交付最快；B/C 需要自绘 KMS 胶水或换 UI 框架，属于 M3 优化。

**迁移触发条件**：若实测 Xorg 启动 > 5 s、RSS > 80 MB，或预览在 A 下因 X11/GLX 路径不稳定，则迁 B（保留 egui，自绘 GBM/EGL）或 C（Slint linuxkms-software）。

### 5.2 最小 spike（全部用板上已有工具，不装新包）

**S0. 只读确认（1 分钟）**

```bash
cat /etc/ld.so.conf.d/00-aarch64-mali.conf
ls -l /usr/lib/aarch64-linux-gnu/mali/
grep -E 'glamor|AIGLX|GLX|Modeline' /var/log/Xorg.0.log
```

预期：mali 目录优先；`mali/libEGL.so.1`、`mali/libgbm.so.1` 存在；日志出现 `glamor X acceleration enabled on Mali-G52` 与 `GLX: Initialized DRISWRAST`。

**S1. EGL 平台探针（10 分钟，判定 (a) 与 Path B）**

把下面保存为 `/tmp/egl_probe.py`，在 **没有 Xorg 占用 DRM** 时运行：

```python
import ctypes as C, os, sys

MALI = "/usr/lib/aarch64-linux-gnu/mali/"
egl = C.CDLL(MALI + "libEGL.so.1")
egl.eglGetProcAddress.restype = C.c_void_p
egl.eglGetProcAddress.argtypes = [C.c_char_p]
egl.eglQueryString.restype = C.c_char_p
egl.eglQueryString.argtypes = [C.c_void_p, C.c_int]
egl.eglInitialize.restype = C.c_uint
egl.eglInitialize.argtypes = [C.c_void_p, C.POINTER(C.c_int), C.POINTER(C.c_int)]
egl.eglGetError.restype = C.c_int

print("client extensions:", (egl.eglQueryString(None, 0x3055) or b"").decode(errors="replace"))
p = egl.eglGetProcAddress(b"eglGetPlatformDisplayEXT")
print("eglGetPlatformDisplayEXT:", bool(p))
if not p:
    sys.exit("no eglGetPlatformDisplayEXT")
gpd = C.CFUNCTYPE(C.c_void_p, C.c_uint, C.c_void_p, C.c_void_p)(p)

def probe(name, platform, native):
    d = gpd(platform, native, None)
    maj, mnr = C.c_int(), C.c_int()
    ok = egl.eglInitialize(d, C.byref(maj), C.byref(mnr)) if d else 0
    if ok:
        print(f"{name}: display={d} init=1 version={egl.eglQueryString(d,0x3054)} vendor={egl.eglQueryString(d,0x3053)}")
    else:
        print(f"{name}: display={d} init=0 err=0x{egl.eglGetError():x}")

# GBM
gbm = C.CDLL(MALI + "libgbm.so.1")
gbm.gbm_create_device.restype = C.c_void_p
gbm.gbm_create_device.argtypes = [C.c_int]
fd = os.open("/dev/dri/card0", os.O_RDWR)
dev = gbm.gbm_create_device(fd)
print("gbm device:", dev)
probe("GBM", 0x31D7, dev)

# X11（有 Xorg 时）
x11 = C.CDLL("libX11.so.6")
x11.XOpenDisplay.restype = C.c_void_p
x11.XOpenDisplay.argtypes = [C.c_char_p]
xd = x11.XOpenDisplay((os.environ.get("DISPLAY") or ":0").encode())
print("X display:", xd)
if xd:
    probe("X11", 0x31D5, xd)
```

判定：
- `GBM: init=1` 且 `vendor` 含 `ARM`/`Mali` → **Path B 可行，无需新包**；
- `GBM: init=0` → 需要 `libmali-bifrost-g52-g2p0-gbm`（13.5 MB）或退回 A；
- `client extensions` 里是否有 `EGL_EXT_platform_x11` / `EGL_KHR_platform_x11`，决定 wgpu 能否直接用 X11 平台显示。

> 若 `eglGetPlatformDisplayEXT` 不可用，按 kmscube 的回退逻辑用 `eglGetDisplay((EGLNativeDisplayType)dev)` 再试一次。

**S2. KMS + dumb buffer + 30 fps 实测（5 分钟，判定 (b)/Path C）**

先释放 DRM master（会中断当前 X 会话）：

```bash
sudo pkill Xorg 2>/dev/null || true
timeout 15 gst-launch-1.0 -v videotestsrc num-buffers=450 ! \
  video/x-raw,width=480,height=800,framerate=30/1 ! \
  fpsdisplaysink video-sink=kmssink sync=false
```

预期：满屏彩条 15 s，`fpsdisplaysink` 报告约 30 fps（或更高），无 `Could not become DRM master`。
可选旋转（DRM rotation 位图，90° 通常为 2；不同 VOP 平面能力不同）：

```bash
timeout 10 gst-launch-1.0 videotestsrc ! video/x-raw,width=480,height=800 ! \
  kmssink plane-properties=s,rotation=2 sync=false
```

若 `kmssink` 失败：`gst-inspect-1.0 kmssink` 查属性/连接器；`SLINT_DRM_OUTPUT=list` 可在 Slint spike 中列出输出。

**S3. Path A 的 egui/wgpu 冒烟（30 分钟，需交叉编译）**

```bash
# 交叉编译后，在板上：
DISPLAY=:0 WGPU_BACKEND=gl RUST_LOG=wgpu_hal=debug ./tspi-cam-ui
```

检查：
- 日志出现 `Using X11 platform`（wgpu EGL）而不是 GLX；
- 适配器名包含 `Mali-G52`，不是 `llvmpipe`；
- 若出现 `failed to get X11 display` 或 adapter 为 `llvmpipe`，改用 Path B 的自定义 EGL，或临时验证 `glow` + `EglThenGlx` 是否可强制 EGL。

**S4. 启动/内存测量（5 分钟）**

```bash
/usr/bin/time -v ./tspi-cam-ui 2>&1 | grep -E 'Elapsed|Maximum resident'
# 另一终端：
ps -o pid,rss,cmd -C tspi-cam-ui
```

与 Xorg 对比：

```bash
/usr/bin/time -v Xorg :1 -nolisten tcp -config /etc/X11/xorg.conf.d/20-modesetting.conf
```

---

## 6. 风险与未验证清单

1. **`libmali` x11 变体的 GBM 无 X 初始化**：强迹象但未直接验证；S1 是决定性测试。
2. **wgpu GL 后端 vs libmali 扩展**：wgpu 需要 `EGL_EXT_platform_x11` 或回退 `eglGetDisplay`；GLES 3.x 扩展是否齐全未核实。
3. **Xorg 无 WM**：全屏/焦点/光标隐藏需要手动处理；触摸事件在 winit 下的坐标映射需实测。
4. **Xorg 成本**：启动约 3.7 s 已测；RSS 未测；`FlipFB=always` 可能降低吞吐。
5. **纯 KMS 软绘制性能**：480×800@30 的扫描带宽足够，但 MJPEG 解码 + 旋转 + letterbox 的 CPU 预算未测。
6. **Slint 许可证**：`GPL-3.0-only OR Royalty-free-2.0 OR Commercial`；若项目要保持 MIT/Apache，需评估 Royalty-free 条款或放弃。
7. **`libseat` 缺失**：Slint `backend-linuxkms` 默认组合在 1.18 前隐含 libseat；在板上要显式用 `backend-linuxkms-libinput`（无 libseat），并以 root 或 `video`/`input` 组运行。
8. **`modetest`/`kmscube` 未安装**：不要假设它们可用；用 `kmssink` 或 Slint 的 `SLINT_DRM_OUTPUT=list` 做枚举。
9. **板上没有 gcc**：所有 C 示例（kmscube、modetest）只能交叉编译后拷贝，或改用 GStreamer/Slint 的预编译二进制；ctypes 探针是唯一“零编译”验证手段。

---

## 7. 附：本次使用的证据链接（精选）

- EGL GBM 规范：https://registry.khronos.org/EGL/extensions/KHR/EGL_KHR_platform_gbm.txt
- kmscube GBM/EGL 初始化：https://gitlab.freedesktop.org/mesa/kmscube/-/blob/master/common.c
- Xorg glamor 的 GBM 初始化：https://gitlab.freedesktop.org/xorg/xserver/-/blob/master/glamor/glamor_egl.c
- drm-rs DumbBuffer：https://github.com/smithay/drm-rs/blob/master/src/control/dumbbuffer.rs
- gbm.rs README（scanout + set_crtc）：https://github.com/Smithay/gbm.rs
- kmssink 文档（plane rotation / can-scale）：https://gstreamer.freedesktop.org/documentation/kms/index.html
- GStreamer KMS dumb buffer 分配器：https://github.com/GStreamer/gst-plugins-bad/blob/master/sys/kms/gstkmsallocator.c
- Slint LinuxKMS 文档（software=dumb buffer；femtovg=GBM/EGL；libinput/libseat 可选）：https://docs.slint.dev/latest/docs/slint/guide/backends-and-renderers/backend_linuxkms
- wgpu GLES/EGL X11 平台：https://github.com/gfx-rs/wgpu/blob/trunk/wgpu-hal/src/gles/egl.rs
- dumb buffer 读写性能实测：https://eighty-twenty.org/2025/03/06/kms-drm-dumb-buffers
- Mesa Panfrost 要求：https://docs.mesa3d.org/drivers/panfrost.html
- libmali 包/变体：https://github.com/tsukumijima/libmali-rockchip/blob/master/debian/targets
