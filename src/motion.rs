//! Generated from the design kit; edit the kit, not this file.
pub mod duration {
    pub const HOVER: std::time::Duration = std::time::Duration::from_millis(150);
    pub const ENTER: std::time::Duration = std::time::Duration::from_millis(150);
    pub const EXIT: std::time::Duration = std::time::Duration::from_millis(100);
    pub const EMPHASIS: std::time::Duration = std::time::Duration::from_millis(300);
    pub const BUMP: std::time::Duration = std::time::Duration::from_millis(350);
    pub const FILL: std::time::Duration = std::time::Duration::from_millis(400);
    pub const CONFIRM: std::time::Duration = std::time::Duration::from_millis(1000);
    pub const LOCATE: std::time::Duration = std::time::Duration::from_millis(1200);
}
pub mod easing {
    pub const STANDARD: [f32; 4] = [0.25, 0.1, 0.25, 1.0];
    pub const ENTER: [f32; 4] = [0.0, 0.0, 0.58, 1.0];
    pub const EXIT: [f32; 4] = [0.42, 0.0, 1.0, 1.0];
}
pub mod distance {
    pub const ENTER_PX: f32 = 10.0;
}
pub mod scale {
    pub const BUMP: f32 = 1.4;
}
pub mod focus {
    pub const WIDTH_PX: f32 = 2.0;
    pub const OFFSET_PX: f32 = 2.0;
}

/// Debug builds only: MOTION_TIME_SCALE=10 slows every duration tenfold so motion can be recorded.
pub fn scaled(d: std::time::Duration) -> std::time::Duration {
    if cfg!(debug_assertions) {
        if let Some(s) = std::env::var("MOTION_TIME_SCALE").ok().and_then(|v| v.parse::<f32>().ok()).filter(|s| *s > 0.0) {
            return d.mul_f32(s);
        }
    }
    d
}
