//! `actiond.toml` parsing and validation. Mirrors the template in
//! `config/actiond.toml`. See docs/architecture.md §12.

use serde::Deserialize;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("cannot read config file {0}: {1}")]
    ReadFailed(String, String),
    #[error("cannot parse config file: {0}")]
    ParseFailed(String),
    #[error("invalid config: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub camera: CameraConfig,
    #[serde(default)]
    pub imu: ImuConfig,
    #[serde(default)]
    pub record: RecordConfig,
    #[serde(default)]
    pub preview: PreviewConfig,
    #[serde(default)]
    pub ipc: IpcConfig,
    #[serde(default)]
    pub log: LogConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CameraConfig {
    #[serde(default = "default_camera_device")]
    pub device: String,
    #[serde(default = "default_width")]
    pub width: u32,
    #[serde(default = "default_height")]
    pub height: u32,
    #[serde(default = "default_fps")]
    pub fps: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImuConfig {
    #[serde(default = "default_imu_device")]
    pub device: String,
    #[serde(default = "default_rate_hz")]
    pub rate_hz: u32,
    #[serde(default = "default_max_consecutive_dropped")]
    pub max_consecutive_dropped: u64,
    #[serde(default = "default_max_dropped_per_10s")]
    pub max_dropped_per_10s: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RecordConfig {
    #[serde(default = "default_record_dir")]
    pub dir: String,
    #[serde(default = "default_headroom_mb")]
    pub storage_headroom_mb: u64,
    #[serde(default = "default_video_queue_max_frames")]
    pub video_queue_max_frames: usize,
    #[serde(default = "default_video_queue_max_bytes_mb")]
    pub video_queue_max_bytes_mb: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PreviewConfig {
    #[serde(default = "default_preview_max_fps")]
    pub max_fps: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IpcConfig {
    #[serde(default = "default_control_socket")]
    pub control_socket: String,
    #[serde(default = "default_preview_socket")]
    pub preview_socket: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LogConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
}

fn default_camera_device() -> String {
    "/dev/video0".to_string()
}
fn default_width() -> u32 {
    1920
}
fn default_height() -> u32 {
    1080
}
fn default_fps() -> u32 {
    30
}
fn default_imu_device() -> String {
    "/dev/spidev3.0".to_string()
}
fn default_rate_hz() -> u32 {
    200
}
fn default_max_consecutive_dropped() -> u64 {
    50
}
fn default_max_dropped_per_10s() -> u64 {
    100
}
fn default_record_dir() -> String {
    "/userdata/rec".to_string()
}
fn default_headroom_mb() -> u64 {
    500
}
fn default_video_queue_max_frames() -> usize {
    8
}
fn default_video_queue_max_bytes_mb() -> usize {
    64
}
fn default_preview_max_fps() -> u32 {
    10
}
fn default_control_socket() -> String {
    "/run/tspi-cam/control.sock".to_string()
}
fn default_preview_socket() -> String {
    "/run/tspi-cam/preview.sock".to_string()
}
fn default_log_level() -> String {
    "info".to_string()
}

impl Default for CameraConfig {
    fn default() -> Self {
        Self {
            device: default_camera_device(),
            width: default_width(),
            height: default_height(),
            fps: default_fps(),
        }
    }
}

impl Default for ImuConfig {
    fn default() -> Self {
        Self {
            device: default_imu_device(),
            rate_hz: default_rate_hz(),
            max_consecutive_dropped: default_max_consecutive_dropped(),
            max_dropped_per_10s: default_max_dropped_per_10s(),
        }
    }
}

impl Default for RecordConfig {
    fn default() -> Self {
        Self {
            dir: default_record_dir(),
            storage_headroom_mb: default_headroom_mb(),
            video_queue_max_frames: default_video_queue_max_frames(),
            video_queue_max_bytes_mb: default_video_queue_max_bytes_mb(),
        }
    }
}

impl Default for PreviewConfig {
    fn default() -> Self {
        Self {
            max_fps: default_preview_max_fps(),
        }
    }
}

impl Default for IpcConfig {
    fn default() -> Self {
        Self {
            control_socket: default_control_socket(),
            preview_socket: default_preview_socket(),
        }
    }
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
        }
    }
}

impl Config {
    /// Load from a TOML file, falling back to defaults for missing keys.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| ConfigError::ReadFailed(path.display().to_string(), e.to_string()))?;
        let cfg: Config =
            toml::from_str(&text).map_err(|e| ConfigError::ParseFailed(e.to_string()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Reject values that cannot work; the daemon refuses to start on them.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.record.dir == "/" {
            return Err(ConfigError::Invalid("record.dir must not be /".to_string()));
        }
        if self.camera.width == 0 || self.camera.height == 0 || self.camera.fps == 0 {
            return Err(ConfigError::Invalid(
                "camera width/height/fps must be non-zero".to_string(),
            ));
        }
        if self.imu.rate_hz == 0 {
            return Err(ConfigError::Invalid(
                "imu.rate_hz must be non-zero".to_string(),
            ));
        }
        Ok(())
    }
}
