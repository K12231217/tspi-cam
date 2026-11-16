# MPP 硬件编码：板级事实与实现（2026-11-16 已跑通）

- 板：`k@192.168.31.129`（Ubuntu 20.04.6，kernel 4.19.232 aarch64）
- 库：`librockchip_mpp` 1.5.0-1（BSP 包），头文件同版本
- 结论：**VEPU2 硬编已接入 `action-core::encode::mpp`，1080p 与 5MP 都出正确分辨率的 H.264**。

## 关键坑（唯一真正卡住的地方）

`MPP_ENC_SET_CFG` 只在 cfg **同时包含 `prep:hor_stride` 和 `prep:ver_stride`** 时成功；
少了任何一个，它返回 `-6 (MPP_ERR_VALUE)`，而 `mpp_enc_cfg_get_s32` 读回的值却是正确的
（`prep:width=1920`），所以看起来像"配置已生效但被忽略"。失败后编码器保持默认 1280x720，
后续再调用 `SET_CFG` 返回 0 也不会改变尺寸。

验证矩阵（`/tmp/kv`，每次全新 ctx）：

| cfg 内容 | SET_CFG | 编码器实际 |
|---|---|---|
| `prep:width,height,format` | -6 | 1280x720 |
| 上面 + `prep:hor_stride,ver_stride` | **0** | **1920x1080** |
| 上面 + rc 全套（bps/fps/gop） | 0 | 1920x1080（5MP 同样成立） |

结论：**传帧时必须显式给 MPP 步幅**，不能指望它从 width 推导。`action-core::encode::mpp`
已按此实现，并在注释里写明原因。

## 其它已确认的板级事实

| 项 | 事实 |
|---|---|
| `MppApi` 表 | `size=184`；`encode_put_frame` 偏移 40、`encode_get_packet` 48、`control` 112（逐项调用验证） |
| 空 cfg | `SET_CFG` 返回 0（说明命令与参数类型正确，失败来自内容） |
| `encode_get_packet` | 队列空时返回 `ret=0` 且 `packet=NULL`，**必须判空结束**，否则段错误 |
| `MPP_ENC_SET_CFG` 数值 | `0x00320001`；`GET_CFG` `0x00320002`；`SET_IDR_FRAME` `0x00320009` |
| 关键帧标志 | `mpp_packet_get_flag` 在 IDR 首包上返回 0，不可依赖；改为扫描 Annex-B NAL：H.264 type 5、H.265 19/20 |
| 输入内存 | v1 是 `mpp_buffer` copy（每帧一次 memcpy）；dmabuf 导入留 v2 |
| 缓冲区组 | `MPP_BUFFER_TYPE_DRM` + `MPP_BUFFER_INTERNAL` 创建成功，无需 root（`k` ∈ video） |

## 板上实测（本仓代码）

```text
./encode_probe /dev/video0 1920 1080 30 10000 30 /tmp/out1080.h264
capture 1920x1080 stride=1920 vstride=1080
encoder open: H264
done: 30 frames -> 31 packets (1 key) 891493 B in 1.065s = 28.16 fps
ffprobe: h264 1920x1080 30/1

./encode_probe /dev/video0 2592 1944 30 32000 20 /tmp/out5mp.h264
done: 20 frames -> 21 packets 1737258 B in 0.898s = 22.28 fps
ffprobe: h264 2592x1944 30/1
```

- 1080p 约 28 fps（采集上限 27–28 fps，编码不是瓶颈）。
- **5MP 只有 22.3 fps**，低于之前 gst 管线实测的 30.045 fps。待查：每帧 7.5 MB 的 memcpy、
  同步 `put_frame`+`get_packet` 逐帧串行、以及单帧码率/RC 设置。编码少量积压后再取包（流水线化）
  预计能补上；这不阻塞功能。

## 构建/运行方式

- 构建：`deploy/container/build.sh --example encode_probe`（Ubuntu 20.04 容器，中文网络需
  `HTTPS_PROXY=http://127.0.0.1:7890`，WSL 镜像网络下可用）。
- 产物直接 scp 到板上运行，不需要 glibc 运行库包（容器链接 2.31）。
- 之前记录的"musl 静态不能 dlopen"与"宿主 2.39 二进制缺 GLIBC_2.32+"均已被容器方案取代。
