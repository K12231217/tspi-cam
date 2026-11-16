# 图形/UI 选型决策（板上实测 + 69 仓库调研收敛）

- 日期：2026-11-16
- 输入：`docs/research/ui-frameworks-35.md`（35 个 UI 仓库）、
  `docs/research/graphics-backends-34.md`（34 个图形后端仓库）、
  `docs/research/mpp-hw-encode.md`（11 个编码路线），合计 69 个 UI/图形仓库 + 11 个编码资料。
- 本文只写**在本板上实测到的证据**与据此收敛的结论；仓库调研细节见上述三份报告。

## 1. 板上实测（本会话，可复核）

| 项 | 结果 | 命令/方法 |
|---|---|---|
| 无 X 时 GBM 平台 EGL | **不可用**：`gbm_create_device(/dev/dri/card0)` 成功，`eglGetPlatformDisplayEXT(EGL_PLATFORM_GBM_KHR)` 返回 NULL | `egl_probe.py`（ctypes） |
| Xorg 下 EGL/GLES | **可用**：EGL 1.4 (ARM) + **GLES 3.2**，`GL_RENDERER=Mali-G52`，`GL_VERSION=OpenGL ES 3.2 v1.g2p0-01eac0` | `egl_x11.py`（ctypes，`DISPLAY=:0`） |
| Xorg 开销 | 常驻 RSS **94 MB**，X0 socket 在 < 2 s 内就绪；`xrandr` 显示 `HDMI-1 480x800+0+0 60.00*` | `Xorg :0 -ac -nolisten tcp vt7` |
| Xorg 下输入 | 触摸被 libinput 识别为 pointer（id=7 `WaveShare WS170120`）；uinput 注入 MT 事件后 X 侧收到 **RawTouchUpdate / RawTouchEnd** | `xinput test-xi2 --root` + 虚拟触摸设备 |
| libmali 变体 | 安装的是 `libmali-bifrost-g52-g2p0-**x11**`；GBM 变体以独立包名存在（`libmali-rockchip` 的 `-gbm` 资产），**未安装** | `dpkg -l`、包文件列表 |
| GLX 状况 | Xorg 日志：`AIGLX error: Calling driver entry point failed` → GLX 回退 `swrast`（**GLX 是软渲染**） | `/var/log/Xorg.0.log` |

## 2. 结论（按优先级）

1. **M2 采用：Xorg + libmali EGL + egui/eframe（glow 渲染器）**。
   - 理由：不需要装新包；`egui_glow` 只依赖 EGL 上下文，不碰 wgpu/Vulkan（板上 Vulkan 仅 lavapipe 软渲染）；
     触摸与显示已在 X11 下实测可用；Xorg 94 MB / < 2 s 启动在 2 GB 板上可接受。
   - 硬约束：**必须走 EGL（`glutin` 的 EGL 后端 / `khronos-egl`），不能走 GLX**（GLX 已确认是软渲染）。
2. **备选 B（需要无 X 时）**：安装 `libmali-…-**gbm**` 变体包，再自建 EGL/GBM 宿主（egui_glow 的 Painter 与窗口系统解耦）。
   本板当前未安装该变体，`UNVERIFIED`；切换成本 = 一个 .deb + 一份 100 行宿主代码。
3. **备选 C（最保守）**：纯 DRM/KMS dumb buffer + `tiny-skia` 软绘制（或 Slint `linuxkms-software`）。
   480×800×4B@30 = 46 MB/s，带宽无压力；已知坑：dumb buffer 读取极慢，只能只写不读。
4. **不采用**：Wayland（板上无合成器）、GTK4/FLTK/Skia（依赖面与交叉编译成本不成比例）、
   druid/xilem/makepad（停更或实验态）、winit+wgpu 的 Vulkan 后端（只有 lavapipe 软渲染）。

## 3. 需要先做的 spike（1 天，失败判据写死）

1. 最小 `eframe` 程序（480×800，一屏彩条 + 一个按钮），跑在 `Xorg :0` 上：
   - 通过：连续 60 s 平均 ≥ 15 fps、RSS ≤ 150 MB、触摸点击产生按钮事件且坐标正确（0..481 / 0..801）。
   - 失败：黑屏 / fps < 8 / 触摸无事件 → 转备选 B 或 C。
2. 同一程序切到 `egui_glow` + 自建 EGL（验证“Painter 可离开 winit”）——只有决定做无 X 版本时才需要。
3. 预览路径：相机 NV12 →（每帧一次缩放/旋转 90°）→ 纹理。缩放/旋转若用 CPU 会吃掉 1080p 的一核，
   优先用 RGA（`librga`）或让 encoder 侧输出小尺寸；UI 只显示 480×800 缩放图。

## 4. 与录制链路的关系

- 编码已定为 MPP 硬编（`mpph264enc`/`mpph265enc`），实测：纯编码 1080p H.264/H.265 各 **36 fps**；
  相机→编码 1080p 持续 **27.0 fps**（300 帧/11.13 s，含启动开销）、120 帧 23.6 fps；
  写出的 MKV 经 `ffprobe` 校验为 `h264 1920x1080 30/1`，抽帧可正常解码（彩条）。
- 编码器插件在**非 root** 用户下不可见，原因不是插件缺功能，而是 `/dev/mpp_service` 为 `root:video 0660`：
  `sudo gst-inspect-1.0 rockchipmpp` 能看到 `mpph264enc/mpph265enc/mppvp8enc/mppjpegenc`。
  利用户 `k` 需要加入 `video` 组（或 udev 放宽权限），这与相机、DRM 的权限问题同源。

## 5. 遗留风险

- X11 下 `FlipFB=always`（板级 xorg.conf 注释提到）可能带来最高 50% 吞吐损失，spike 时用实测 fps 判定。
- 触摸在 X 中呈现为 pointer：单点点击可用，多点手势需另做（M2 不做手势）。
- `libmali` 是 ARM EULA 闭源 blob，升级内核/镜像时需同步匹配；不引入第二个图形栈。
