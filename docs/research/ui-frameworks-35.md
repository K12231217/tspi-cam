# Rust UI 技术栈选型调研（tspi-cam / RK3566 + Mali-G52 + Xorg-only）

> 调研日期：2026-11-16。只读调研，未改动仓库代码。
> 标记约定：**VERIFIED** = 有链接/文档/源码证据；**UNVERIFIED** = 推断或未在板卡上实测。
> 本文的“板上实测”一律标 UNVERIFIED：本轮没有跑板卡。

## 0. 摘要

- 板子上**唯一现成可用的硬件 GL 路径**是 `libmali`（X11 变体）+ `libEGL`，通过 **EGL**（不是 GLX）在 Xorg 下拿到 GLES 3.2。VERIFIED：目标环境已知包含 `libmali-bifrost-g52-g2p0-x11`；libmali 的 x11 / gbm / wayland-gbm 是**三个不同的二进制包**（见 §5）。
- 因此**主选 = egui + eframe 的 glow 渲染器，跑在 Xorg + EGL(libmali)**：egui 已是本项目 `docs/architecture.md` §14 的 M2 计划，`egui_glow` 走 `glow`+`glutin(EGL)`，不碰 wgpu/Vulkan（板上 Vulkan 只有 lavapipe 软渲染）。
- **关键风险 1**：X11 下触摸可能只产生 `CursorMoved` 而不产生 `ButtonPress`（winit issue #3924，VERIFIED），需 evdev→uinput 小工具或 XInput2 方案兜底。
- **关键风险 2**：`libmali` 只装了 **x11 变体**，所以“无 Xorg 的 DRM/KMS + GBM + EGL”这条硬件加速路线**当前不可用**，必须先装 gbm 变体（VERIFIED 该包存在，UNVERIFIED 在板上可跑）。
- 结论：**egui 做 UI 层，平台宿主可替换**（`eframe`(winit/X11) 今天 → 自建 DRM/GBM/EGL host 以后）。纯软件兜底 = `tiny-skia` + `softbuffer`（X11）或 `slint linuxkms-software`（DRM 直出）。

## 1. 结论表（35 项）

| # | 仓库 | ★ | 许可证 | 最近 release / 提交 | 后端（X11 / DRM-KMS / Wayland / 软件） | aarch64 / 嵌入式证据 | 结论 | 一句话理由 |
|---|------|---|--------|--------------------|----------------------------------------|----------------------|------|-----------|
| 1 | [emilk/egui](https://github.com/emilk/egui)（eframe） | 32k | MIT OR Apache-2.0 | v0.37.0（2026-11-13 提交） | X11 ✅ / DRM ❌ / Wayland ✅ / 软件 ❌ | 纯 Rust；winit X11 + EGL 均运行时 dlopen；本板实测 **UNVERIFIED**（官方“无合成器”问答 [discussion #5642](https://github.com/emilk/egui/discussions/5642) 至今 0 回复） | **采用（主）** | 计划内技术栈、glow 可直连 libmali EGL/GLES、无 C 依赖、交叉编译最省事 |
| 2 | egui-wgpu（egui 子 crate） | 32k | MIT OR Apache-2.0 | v0.37.0 | X11 ✅ / DRM ❌ / Wayland ✅ / 软件 ❌ | wgpu GL 后端存在（[wgpu_hal::gles](https://wgpu.rs/doc/wgpu_hal/gles/index.html)）但板上只有 lavapipe Vulkan；**UNVERIFIED** | 备选（默认关闭） | wgpu-GL 在 Mali 上有已知渲染缺陷（[egui #8012](https://github.com/emilk/egui/issues/8012)），收益不抵风险 |
| 3 | egui_glow | 32k | MIT OR Apache-2.0 | v0.37.0 | X11 ✅（EGL/GLES）/ DRM ✅（只要有 EGL 上下文）/ Wayland ✅ / 软件 ❌ | `Painter` 与窗口系统解耦，可挂任意 GL 上下文（[docs](https://docs.rs/egui_glow/latest/egui_glow/)）；**UNVERIFIED** 板上 | **采用** | 与 #1 组合；未来换成自建 DRM/GBM/EGL 宿主时这一层不用改 |
| 4 | [slint-ui/slint](https://github.com/slint-ui/slint) | 24.6k | GPLv3 或商业（嵌入式商业需版税） | v1.18.1（2026-11-16 提交） | X11 ✅ / **DRM ✅（linuxkms）** / Wayland ✅ / 软件 ✅（DRM dumb buffer + linuxfb） | **VERIFIED**：官方文档列 ARM64 无 GPU 软件渲染容器 + i.MX8/AM62 GPU 容器，`backend-linuxkms` 文档写明 DRM/libseat/libinput；仓库有 `docker/Dockerfile.aarch64-unknown-linux-gnu` | 备选（第 3 条路线） | 唯一“框架级”原生 DRM/KMS 方案，但 GPLv3 会传染本仓库 MIT/Apache，且要 fontconfig + 独立 DSL |
| 5 | [iced-rs/iced](https://github.com/iced-rs/iced) | 32k | MIT | v0.14.0 | X11 ✅ / DRM ❌ / Wayland ✅ / 软件 ✅（`tiny-skia` renderer） | `Cargo.toml` 明确 `default = ["wgpu","tiny-skia","x11","wayland",…]`；纯 Rust，无 C 依赖；板上 **UNVERIFIED** | 备选 | 软件渲染 + 控件齐全，是 egui 之外的“无 GPU”可控方案；但 Elm 架构与调试成本高于 egui |
| 6 | [fltk-rs/fltk-rs](https://github.com/fltk-rs/fltk-rs) | 1.9k | MIT（FLTK 本体 LGPL-2.0+exception） | v1.5.23 | X11 ✅ / DRM ❌（无 KMS 后端）/ Wayland 部分 / 软件 ✅ | 需编译 C++ FLTK，交叉编译要目标 sysroot；**UNVERIFIED** | 排除 | 引入 C++ 构建链，与“交叉编译省事、依赖少”冲突 |
| 7 | [gtk-rs/gtk4-rs](https://github.com/gtk-rs/gtk4-rs) | 2.4k | MIT（GTK 本体 LGPL-2.1） | v0.11.5 | X11 ✅ / DRM ❌ / Wayland ✅ / 软件 ✅（cairo） | Ubuntu arm64 有 gtk4 包（**UNVERIFIED 本板已装**） | 排除 | 依赖面巨大（glib/pango/cairo/gdk-pixbuf…），启动慢、内存高，480×800 小 UI 严重过度 |
| 8 | [Relm4/relm4](https://github.com/Relm4/relm4) | 2k | MIT | v0.11.0 | 同 GTK4 | 同 GTK4，**UNVERIFIED** | 排除 | 只是 GTK4 的 Elm 封装，继承 GTK 全部部署成本 |
| 9 | [linebender/druid](https://github.com/linebender/druid) | 9.7k | Apache-2.0 | v0.8.3（已停止开发，转向 xilem） | X11 ✅ / DRM ❌ / Wayland ✅ / 软件 ❌ | **UNVERIFIED** | 排除 | 项目已归档式停更，无安全/兼容维护 |
| 10 | [makepad/makepad](https://github.com/makepad/makepad) | 7.5k | MIT | crates.io 0.1.0，git 活跃 | X11 ✅ / Wayland ✅ / DRM ？ / 软件 ❌ | Linux 依赖列表含 `libdrm-dev libgbm-dev libegl1…`（[README](https://github.com/makepad/makepad)）但**未见 linuxkms 平台代码证据**，**UNVERIFIED** | 排除（观望） | 自带 DSL + 自研 shader 渲染器，生态/文档不足以承载交付；DRM 支持不可核实 |
| 11 | [marc2332/freya](https://github.com/marc2332/freya) | 3.4k | MIT | v0.5.2 | X11 ✅ / DRM ❌ / Wayland ✅ / 软件 ✅（Skia CPU） | 走 skia-safe：**VERIFIED** 0.157.0 有 `aarch64-unknown-linux-gnu-…-x11` 预编译包（[assets](https://github.com/rust-skia/skia-binaries/releases/expanded_assets/0.157.0)） | 排除 | 拖入 ~20 MB C++ Skia + libstdc++，对 3 个按钮的 UI 不成比例 |
| 12 | [linebender/xilem](https://github.com/linebender/xilem) | 5.6k | Apache-2.0 | 无正式 release（2026-11-03 提交） | X11 ✅ / DRM ❌ / Wayland ✅ / 软件 ❌ | winit + Vello + wgpu，Linux 需 `clang/libvulkan-dev`（README）；**UNVERIFIED** | 排除 | 官方自述 “experimental”，API 未稳定，且强依赖 GPU 计算路径 |
| 13 | [tauri-apps/tauri](https://github.com/tauri-apps/tauri) | 113k | MIT OR Apache-2.0 | v2.x（v3 alpha） | X11 ✅ / DRM ❌ / Wayland ✅ / 软件 ❌ | Linux 依赖 WebKitGTK 4.1（[prerequisites](https://v2.tauri.app/start/prerequisites/)）；arm64 有发行版包但**UNVERIFIED 本板** | 排除 | WebKitGTK 体量/内存/启动时间与“2 GB 内存、启动快”直接冲突 |
| 14 | [tauri-apps/wry](https://github.com/tauri-apps/wry) | 5.1k | MIT OR Apache-2.0 | wry-v0.57.2 | 同 WebKitGTK | **UNVERIFIED** | 排除 | 同上，且需要 JS/HTML 层，调试面更大 |
| 15 | [emoon/rust_minifb](https://github.com/emoon/rust_minifb) | 1.2k | MIT OR Apache-2.0 | v0.29.0（2026-10-03） | X11 ✅ / DRM ❌ / Wayland ✅ / 软件 ✅ | 依赖 `x11-dl`/`wayland-client`（[crate 页](https://docs.rs/crate/minifb/latest)），纯 Rust；**UNVERIFIED 板上** | 备选（极简自绘） | 只给“一块像素缓冲”，做 3 按钮 UI 要自绘控件；远小于 egui 但功能也远小于 |
| 16 | [parasyte/pixels](https://github.com/parasyte/pixels) | 2.2k | MIT | v0.17.2 | X11 ✅ / DRM ❌ / Wayland ✅ / 软件 ❌（需 GPU） | README：仅 wgpu（Vulkan/Metal/DX12/GLES3）；板上 Vulkan=lavapipe，**UNVERIFIED** | 排除 | 需要 GPU 但本板 Vulkan 只有软实现，等于自找 60 fps 之外的额外开销 |
| 17 | [rust-windowing/softbuffer](https://github.com/rust-windowing/softbuffer) | 514 | MIT OR Apache-2.0 | v0.4.8 | X11 ✅（XCB/Xlib）/ **DRM ❌** / Wayland ✅ / 软件 ✅ | 后端表明确只有 XCB/Xlib/Wayland/Windows/macOS/Web（[docs](https://docs.rs/softbuffer/latest/softbuffer/)）；**UNVERIFIED 板上** | 备选（纯软件宿主） | 零 GPU 依赖、纯 Rust；做 CPU 渲染兜底最省心，但没有控件层 |
| 18 | [rust-windowing/winit](https://github.com/rust-windowing/winit) | 6.3k | MIT OR Apache-2.0 | v0.30.13 | X11 ✅ / **DRM ❌** / Wayland ✅ / 软件 n/a | KMS/DRM 后端提议 [#1865](https://github.com/rust-windowing/winit/issues/1865) 已关闭未实现；X11 触摸缺陷 [#3924](https://github.com/rust-windowing/winit/issues/3924) | 采用（间接） | eframe 的窗口/输入层；已知触摸坑必须预先验证 |
| 19 | [rust-windowing/glutin](https://github.com/rust-windowing/glutin) | 2.1k | Apache-2.0 | v0.32.3 | X11 ✅（EGL/GLX）/ KMS ❌（需外部 display）/ — / — | EGL display 可外部构造（[docs](https://docs.rs/glutin/latest/glutin/)）；**UNVERIFIED** 本板 | 采用（间接） | 负责在 X11 上建 EGL 上下文，是拿到 libmali 的入口 |
| 20 | [grovesNL/glow](https://github.com/grovesNL/glow) | 1.5k | MIT OR Apache-2.0 | v0.18.0 | 任意（只做 GL/GLES 绑定与函数加载） | 纯绑定；GLES 3.x 覆盖 | 采用（间接） | egui_glow/femtovg 的底座，无平台假设 |
| 21 | [gfx-rs/wgpu](https://github.com/gfx-rs/wgpu) | 19k | MIT OR Apache-2.0 | v30.0.2 | X11 ✅ / DRM ❌ / Wayland ✅ / 软件 ✅(lavapipe) | `wgpu_hal::gles` 支持 EGL 提供的 GLES3（[doc](https://wgpu.rs/doc/wgpu_hal/gles/index.html)）；Mali 上**UNVERIFIED** | 不首选 | 板上首选后端是 lavapipe（CPU），GL 后端在 Mali 上另有 egui #8012 一类缺陷 |
| 22 | [rust-skia/rust-skia](https://github.com/rust-skia/rust-skia) | 1.8k | MIT | v0.157.0（2026-11-03） | X11 ✅(egl/x11) / DRM ❌（gl 特性里 egl 可接 gbm 但无示例）/ Wayland ✅ / 软件 ✅ | **VERIFIED** 预编译含 `aarch64-unknown-linux-gnu-egl-ganesh-gl-…-x11` 等（[assets](https://github.com/rust-skia/skia-binaries/releases/expanded_assets/0.157.0)） | 排除（仅作 freya/slint-skia 依赖） | 20–25 MB C++ 产物 + libstdc++，与“依赖少/启动快”不符 |
| 23 | [RazrFalcon/tiny-skia](https://github.com/RazrFalcon/tiny-skia) | 1.7k | BSD-3-Clause | v0.12.0 | 仅 CPU 光栅（无窗口） | 纯 Rust，无架构假设 | **采用（兜底渲染内核）** | 480×800 全屏重绘 1.5 MB 级像素，CPU 可承受；配 softbuffer/minifb/DRM 都行 |
| 24 | [femtovg/femtovg](https://github.com/femtovg/femtovg) | 958 | MIT OR Apache-2.0 | v0.28.0（2026-10-27） | 需要 GL 上下文（GLES2/3）或 wgpu；无窗口管理 | README：Slint 的默认渲染器；GLES2 后端存在 | 备选 | 若走“自建 EGL 宿主 + 矢量 UI”，它比 egui 更轻；但要自己写控件与事件 |
| 25 | [linebender/piet](https://github.com/linebender/piet) | 1.4k | Apache-2.0 OR MIT | v0.8.0（停更） | 依赖后端（cairo/direct2d/core-graphics） | **UNVERIFIED** | 排除 | 已被 Vello 取代，无维护 |
| 26 | [linebender/vello](https://github.com/linebender/vello) | 4.5k | Apache-2.0 OR MIT | v0.12.0 | X11 ✅(wgpu) / DRM ❌ / Wayland ✅ / 软件 ✅(vello_cpu，新) | README：`vello_gpu` 走 wgpu，无需 compute；`vello`(compute) 仍 experimental；**UNVERIFIED** 板上 | 排除 | GPU 路线依赖 wgpu/lavapipe；`vello_cpu` 尚年轻，不如 tiny-skia 稳 |
| 27 | [pop-os/cosmic-text](https://github.com/pop-os/cosmic-text) | 2.2k | MIT | v0.19.0 | 无窗口（纯文本整形/排版） | 纯 Rust | 采用（支撑库） | 自绘路线的文本层；egui 自带字体栈时不必要 |
| 28 | [alexheretic/ab-glyph](https://github.com/alexheretic/ab-glyph) / rusttype | 455 | MIT OR Apache-2.0 | ab_glyph v0.2.32；rusttype v0.9.3（停更） | 无窗口（字形光栅） | 纯 Rust | 备选（支撑库） | 只在 tiny-skia 自绘里需要；rusttype 已被 ab_glyph 取代 |
| 29 | [Rust-SDL2/rust-sdl2](https://github.com/Rust-SDL2/rust-sdl2)（`sdl2`，含 **kmsdrm**） | 3k | MIT（SDL2 zlib） | sdl2 v0.38.0 | X11 ✅ / **DRM ✅（kmsdrm 视频驱动）** / Wayland ✅ / 软件 ✅ | **VERIFIED** SDL2 源码有 `src/video/kmsdrm/`（含 `SDL_kmsdrmopengles.c`）；需 `--enable-video-kmsdrm` 构建（[社区说明](https://community.st.com/t5/stm32-mpus-products-and-hardware/direct-use-of-sdl-with-kms-drm/td-p/253165)）；**UNVERIFIED 本板包是否启用** | 备选（去 Xorg 的现成路线） | 唯一的“成熟 + DRM 直出 + GLES” C 方案；代价是交叉编译 SDL2 本身 |
| 30 | druid-shell（druid 子 crate） | 9.7k | Apache-2.0 | v0.8.x（随 druid 停更） | X11 ✅ / DRM ❌ / Wayland ✅ / 软件 ❌ | **UNVERIFIED** | 排除 | 依附已停更的 druid |
| 31 | [psychon/x11rb](https://github.com/psychon/x11rb) | 436 | MIT OR Apache-2.0 | v0.14.0 | X11 ✅（协议层，含 SHM） / 其余 ❌ | 纯 Rust X11 绑定 | 备选（原生 X11 兜底） | 若 winit 触摸/EGL 路线受挫，可直接 `SHM PutImage` + XInput2，最可控但自绘量最大 |
| 32 | [Smithay/wayland-rs](https://github.com/Smithay/wayland-rs)（wayland-client） | 1.5k | MIT | v0.31.15 | Wayland ✅ | 纯 Rust | 排除 | 板上没有合成器，也不需要写 Wayland 客户端 |
| 33 | [Smithay/smithay](https://github.com/Smithay/smithay) | 3.4k | MIT | v0.7.0 | Wayland 合成器工具包（可驱动 DRM） | **UNVERIFIED** | 排除 | 等于自己写合成器：工程量与本项目目标不匹配 |
| 34 | [Smithay/drm-rs](https://github.com/Smithay/drm-rs)（`drm`） | 115 | MIT | v0.15.0 | **DRM/KMS ✅（ioctl + dumb buffer）** / 无 GBM/EGL | 纯 Rust，直接 ioctl（[docs](https://docs.rs/drm/latest/drm/)） | 备选（自建 DRM 直出的底座） | 软件直出到面板的最短路径；要 GPU 需另配 `gbm`+`egl` |
| 35 | [bevyengine/bevy](https://github.com/bevyengine/bevy)（bevy_ui，对照） | 50k | MIT OR Apache-2.0 | v0.20.1 | X11 ✅ / DRM ❌ / Wayland ✅ / 软件 ❌（wgpu） | **UNVERIFIED** | 排除（仅对照） | 引擎级体量 + wgpu/lavapipe，做 UI 是反向工程 |

## 2. Top3 推荐（含最小 spike 与失败判据）

### T1（主选）egui v0.37 + eframe(glow only) + Xorg/EGL(libmali)

Cargo 形态（VERIFIED 这些 feature 存在，见 [eframe feature flags](https://docs.rs/eframe/latest/eframe/#feature-flags)）：

```toml
[dependencies]
eframe = { version = "0.37", default-features = false, features = ["glow", "x11", "default_fonts"] }
egui   = "0.37"
```
（关掉默认的 `wgpu` + `wayland`：wgpu 会拉 Vulkan/lavapipe，wayland 会在交叉编译时要求 libwayland 头文件。）

**最小 spike（板上有 X 的前提下，按顺序，每步都是 go/no-go）**

1. 确认 X 与 EGL 供应者：
   `DISPLAY=:0 glxinfo -B`（预期：`Mesa`/llvmpipe，说明 GLX 不走 Mali）
   `DISPLAY=:0 eglinfo -B` 或 `es2_info`（预期：`EGL_VENDOR = ARM`，`GL_RENDERER` 含 `Mali-G52`）。
2. 下最小 Rust 程序：`winit` 建窗口 + `glutin`(EGL) 建 GLES 上下文 + `glow` 清屏，打印 `GL_VERSION/GL_RENDERER/GL_EXTENSIONS`。
   *判定：`GL_RENDERER` 必须含 `Mali`。*
3. 换成 `egui + egui_glow`：一个 480×800 全屏窗口，画文字 + 3 个按钮 + 一张 640×480 纹理，`request_repaint` 固定 30 Hz，打印每帧 CPU 时间。
4. 触摸：`xev -event button` 观察单指点击是否产生 `ButtonPress`/`ButtonRelease`；再在 egui 里点按钮。
5. 预览链路：把 720p MJPEG 单帧解码（`zune-jpeg`）→ RGB → `egui::TextureHandle::set`，30 fps；`top` 看 CPU。
6. 冷启动：`systemd-analyze`/手工 `time` 到首帧；内存 `ps_mem`。

**失败判据（任一命中即转 T2/T3）**
- 步骤 1/2 里 `GL_RENDERER` 不是 Mali（回落到 llvmpipe）：X11+libmali EGL 不可用。
- 步骤 3 稳态帧时间 > 33 ms（< 30 fps）或单帧 CPU > 15 ms（UI 本身，不含解码）。
- 步骤 4 单指点按**不**产生按键事件，且 4 小时内找不到稳定的 XInput2/evdev 兜底。
- 步骤 5 预览+UI 合计 > 60% 单核（会把 MJPEG 编码挤掉）。
- 步骤 6 到首帧 > 8 s（开机到可操作），或常驻 RSS > 250 MB。

### T2（备选 A，去 Xorg）egui + `egui_glow::Painter` + 自建 DRM/KMS+GBM+EGL 宿主

前置：安装 **libmali 的 gbm 变体**（例如 `libmali-bifrost-g52-g2p0-gbm`；此类包在 LinuxFactory/tsukumijima 仓库中确实按 `x11`/`gbm`/`wayland-gbm` 分开发布，见 [LinuxFactory 目录](https://ppa.linuxfactory.or.kr/pool/rockchip/libm/libmali) / [libmali-rockchip releases](https://github.com/tsukumijima/libmali-rockchip/releases)）。代码骨架：`drm`(§34) 或 `libdrm` FFI 做 modeset/page-flip，`gbm` 建 surface，`egl` 建上下文 → 把 `egui_glow::Painter` 画进 FBO → 每帧 `eglSwapBuffers` + atomic commit。输入走 `evdev`/`libinput`（触摸天然正确，绕开 X11 的 XI2 坑）。

**spike**：只做“GBM surface + EGL context + 清屏 + page flip 30 s 不撕裂”，再挂 `egui_glow::Painter` 画同一套 UI。
**失败判据**：`eglGetPlatformDisplay(EGL_PLATFORM_GBM_KHR)` 失败 / 无 `EGL_KHR_platform_gbm`；或只能跑 1 个缓冲导致撕裂且无法用 fence 修复；或必须 root 才能拿 DRM master 而 systemd 服务模型不接受。

### T3（备选 B，无 GPU、无 X）slint `backend-linuxkms` + `renderer-software`

**spike**：`slint = { version="1.18", default-features=false, features=["compat-1-18","backend-linuxkms","backend-linuxkms-libinput","renderer-software"] }`，在 VT 上跑一个 480×800 的按钮页；`SLINT_DRM_MODE=list` 确认只有 480×800@60 一个模式；`SLINT_KMS_ROTATION=0`。
**失败判据**：GPLv3 与本仓库 MIT/Apache 冲突且不接受（**首要否决项**）；fontconfig 缺失或字体渲染不可接受；软件渲染下 30 fps UI + 预览 > 70% 单核；libseat/DRM master 权限在 systemd 下拿不到。

> 若 T1 挂而 T3 因许可证被否：直接用 **T3' = tiny-skia + softbuffer(X11)** 或 **tiny-skia + drm(§34)** 自绘（3 个按钮 + 1 张预览纹理，代码量约 600–900 行，纯 Rust，零 C 依赖、零 GPU 依赖）。

## 3. 被排除项一句话理由

- **fltk-rs**：拖入 C++ FLTK 构建链，交叉编译与“依赖少”目标冲突。
- **gtk-rs / relm4**：GTK4 全家桶体量与启动时间对 2 GB 内存、480×800 的单一用途设备是过度设计。
- **druid / druid-shell / piet**：上游已停更/被 xilem、vello 取代，无维护。
- **makepad**：自研 DSL + 自研渲染器，DRM 支持无法核实，生态不足以承担交付风险。
- **freya / skia-safe(直接依赖)**：为了 3 个按钮拖入 20 MB C++ Skia 与 libstdc++，收益不成比例。
- **xilem**：官方自述 experimental，且强绑 Vello/wgpu。
- **tauri / wry**：WebKitGTK 体量、内存与启动时间全面违背约束。
- **pixels**：必须 GPU，而本板 Vulkan 只有 lavapipe，等于用软渲染换更复杂的栈。
- **vello**：GPU 路径依赖 wgpu；CPU 路径（`vello_cpu`）尚不如 tiny-skia 成熟。
- **wayland-client / smithay**：板上没有合成器，也没有理由自己写一个。
- **bevy_ui**：引擎级体量，仅作对照。

## 4. 三种后端的可行性判定

| 后端 | 硬件加速？ | 现状 | 证据等级 |
|---|---|---|---|
| ① X11 + EGL/GLES(libmali x11 变体) | ✅ Mali-G52 | `libmali.so.1.9.0`(x11) + `libEGL` 已在板；需用 **EGL**（glxinfo 预期仍显示 Mesa） | 包存在 **VERIFIED**；板上 EGL 选中 Mali **UNVERIFIED**（spike 步骤 1–2） |
| ② 无合成器 DRM/KMS + GBM + EGL | ✅ 但需换库 | 只有 **x11 变体** libmali → `EGL_PLATFORM_GBM` 不可用；需装 gbm 变体 | x11/gbm/wayland-gbm 是不同包 **VERIFIED**；gbm 变体在本板可跑 **UNVERIFIED** |
| ③ 纯软件（swrast / tiny-skia / softbuffer / DRM dumb buffer） | ❌ | 完全可行；480×800×4B ≈ 1.5 MB/帧，30 fps ≈ 46 MB/s 内存带宽，不是瓶颈，瓶颈是 CPU 与 MJPEG 编码抢核 | 机制 **VERIFIED**；实测 fps **UNVERIFIED** |

补充事实（对决策有直接影响）：
- **X11 触摸是最大未知数**：winit X11 上单指点按可能只给 `CursorMoved`（[winit #3924](https://github.com/rust-windowing/winit/issues/3924)，关闭原因为“X11 层没产生按键”）。缓解：evdev→uinput 虚拟鼠标小守护，或自绘/自接 XInput2。**该风险对 egui/iced/GTK 一视同仁。**
- **libmali 走 EGL 不走 GLX**：Xorg 的 GLX 由 Mesa 提供（板上 Mesa 无 panfrost 可用路径，因内核用的是瑞芯微私有 `mali400` 模块），因此任何“默认 GLX”的框架（部分 GTK/FLTK 组合）会掉进 llvmpipe。
- **无窗口管理器**：需要自己保证 `480×800` 全屏、位于 (0,0)、无装饰；eframe/winit 可设 inner_size + position，不依赖 WM。
- **面板是竖屏 480×800**：UI 按 480 宽设计；相机预览需 90° 旋转 + letterbox，旋转在 `egui` 侧用纹理 UV/网格变换即可（不要依赖 Xorg rotation，避免触摸坐标二次映射）。

## 5. 证据清单（VERIFIED）

- eframe feature flags（`glow`/`wgpu`/`x11`/`wayland`/`default_fonts`）：https://docs.rs/eframe/latest/eframe/#feature-flags
- egui_glow 与 winit 解耦、`ShaderVersion`/`GlowConfiguration`：https://docs.rs/egui_glow/latest/egui_glow/
- egui 无“无合成器”官方后端（提问 0 回复）：https://github.com/emilk/egui/discussions/5642
- wgpu GLES 后端（EGL）：https://wgpu.rs/doc/wgpu_hal/gles/index.html
- egui + wgpu-GL 文本渲染缺陷：https://github.com/emilk/egui/issues/8012
- slint LinuxKMS 后端（DRM/libseat/libinput，FemtoVG=GLES2.0、Skia、Software）：https://docs.slint.dev/latest/docs/slint/guide/backends-and-renderers/backend_linuxkms
- slint 嵌入式已验证平台（ARM64 无 GPU 软件渲染容器、i.MX8/AM62 GPU 容器）：https://docs.slint.dev/latest/docs/slint/guide/platforms/embedded
- slint 许可证（GPLv3 / 商业，嵌入式商业需版税）：https://slint.dev/pricing
- slint 的 aarch64 交叉 Dockerfile（含“Skia 源码构建”注记）：https://github.com/slint-ui/slint/blob/master/docker/Dockerfile.aarch64-unknown-linux-gnu
- iced 默认 features（`wgpu` + `tiny-skia` + `x11` + `wayland`）：https://github.com/iced-rs/iced/blob/master/Cargo.toml
- winit 平台/feature（仅 x11/wayland）：https://docs.rs/winit/latest/winit/
- winit KMS/DRM 后端提议（已关闭未实现）：https://github.com/rust-windowing/winit/issues/1865
- winit X11 触摸缺陷：https://github.com/rust-windowing/winit/issues/3924
- softbuffer 后端表（无 KMS）：https://docs.rs/softbuffer/latest/softbuffer/
- minifb 0.29.0、X11/Wayland、依赖 x11-dl/wayland-client：https://docs.rs/crate/minifb/latest
- pixels 仅 wgpu（Vulkan/GLES3）：https://github.com/parasyte/pixels
- femtovg 单一 GL(ES)3 后端 + 被 Slint 使用：https://docs.rs/crate/femtovg/latest
- tiny-skia 0.12.0（CPU）：https://docs.rs/tiny-skia
- cosmic-text 0.19.0：https://github.com/pop-os/cosmic-text
- vello 三类渲染器（CPU / GPU(wgpu, 非 compute) / compute 实验）：https://github.com/linebender/vello
- xilem = winit + Vello + wgpu，experimental：https://github.com/linebender/xilem
- skia-safe 0.157.0 **有 aarch64-unknown-linux-gnu 预编译**（含 egl/x11/wayland/vulkan 组合）：https://github.com/rust-skia/skia-binaries/releases/expanded_assets/0.157.0
- skia-safe GPU 特性（gl/vulkan）与源码构建/交叉编译说明：https://docs.rs/skia-safe/latest/skia_safe/ + https://github.com/rust-skia/rust-skia/blob/master/skia-bindings/README.md
- SDL2 KMSDRM 存在（源码含 `SDL_kmsdrmopengles.c`）：https://github.com/libsdl-org/SDL/releases/tag/release-2.32.4 （路径 `src/video/kmsdrm/`）；需 `--enable-video-kmsdrm` 构建：https://community.st.com/t5/stm32-mpus-products-and-hardware/direct-use-of-sdl-with-kms-drm/td-p/253165
- `drm` crate（DRM/KMS ioctl + dumb buffer）：https://docs.rs/drm/latest/drm/
- Tauri Linux 依赖 WebKitGTK 4.1：https://v2.tauri.app/start/prerequisites/
- makepad Linux 依赖清单（含 libdrm/libgbm/libegl）：https://github.com/makepad/makepad
- libmali 按 x11 / gbm / wayland-gbm 分开发布：https://ppa.linuxfactory.or.kr/pool/rockchip/libm/libmali ，https://github.com/tsukumijima/libmali-rockchip/releases
- 直接 DRM+GBM+EGL 的 Rust 极简参考实现（可抄骨架）：https://github.com/ardos-os/easydrm

### UNVERIFIED（必须板上实测）

1. `DISPLAY=:0` 下 `libEGL` 实际选中 libmali（而非 Mesa/llvmpipe），GLES 版本 ≥ 3.0。
2. Xorg 无 WM 时 eframe 全屏窗口、焦点、帧率稳定（含 60 Hz vsync 行为）。
3. 该 USB 触摸在 Xorg 下是否产生 `ButtonPress`（#3924 的复现/排除）。
4. 720p MJPEG 解码 + 旋转 + letterbox 在 30 fps 下的 CPU 占比。
5. `libmali-…-gbm` 变体在本板可提供 `EGL_PLATFORM_GBM` 并完成 page flip。
6. slint linuxkms 软件渲染在本板的实际 fps 与字体质量。
7. 各框架在 `aarch64-unknown-linux-gnu` + zigbuild 下的具体交叉编译产物（本仓库 `.cargo/config.toml` 已固定该 target，UI crate 目前是 327 KB 的 stub）。

## 6. 对本仓库的落地建议

1. **把 UI 层和平台宿主分开**：`runtime/ui` 只依赖 `egui`（+ 一个 `Backend` trait：`present(texture/frame)`、`poll_input()`、`size()`）。宿主可以是 `eframe`(X11/EGL)、自建 DRM/GBM/EGL、或 softbuffer+tiny-skia。
2. **渲染器用 glow，不用 wgpu**：`default-features=false`，只开 `glow`+`x11`+`default_fonts`，避免 Vulkan/lavapipe 与 wayland 头文件。
3. **触摸不要只信 winit**：先做 spike（`xev`）；若失败，加 60–120 行的 evdev→uinput 桥（`/dev/input/event*`，MT-B 协议），X 侧看到的就是普通鼠标，egui 无需改。
4. **预览不要走 UI 线程解码**：`actiond` 已经在推 MJPEG（§9.2 的 preview socket），UI 侧独立线程 `zune-jpeg` 解码 → 只保留最新帧（Latest Frame 槽），`ctx.request_repaint()` 按 30 Hz 驱动。
5. **许可证**：若选 slint，本仓库 MIT/Apache 需整体改为 GPLv3（或购买商业许可）；egui/iced/tiny-skia 无此问题。
6. 决策顺序：**T1 spike（半天）** → 通过则锁定 egui+glow；失败 → **T3'（tiny-skia+softbuffer）**，同时把 T2（装 libmali gbm 变体）作为后续“去 Xorg、加快启动”的独立里程碑。
