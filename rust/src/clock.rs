use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct PlaybackClock {
    start_time: f64,
    pause_offset: f64,
    speed: f64,
    playing: bool,
}

#[wasm_bindgen]
impl PlaybackClock {
    #[wasm_bindgen(constructor)]
    pub fn new() -> PlaybackClock {
        PlaybackClock {
            start_time: 0.0,
            pause_offset: 0.0,
            speed: 1.0,
            playing: false,
        }
    }

    pub fn play(&mut self, now: f64) {
        if !self.playing {
            self.start_time = now - self.pause_offset;
            self.playing = true;
        }
    }

    pub fn pause(&mut self, now: f64) {
        if self.playing {
            self.pause_offset = now - self.start_time;
            self.playing = false;
        }
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    pub fn elapsed_us(&self, now: f64) -> f64 {
        let raw = if self.playing {
            now - self.start_time
        } else {
            self.pause_offset
        };
        raw * 1000.0 * self.speed
    }

    pub fn set_speed(&mut self, now: f64, new_speed: f64) {
        let elapsed = self.elapsed_us(now);
        self.speed = new_speed;
        if self.playing {
            self.start_time = now - elapsed / (new_speed * 1000.0);
        } else {
            self.pause_offset = elapsed / (new_speed * 1000.0);
        }
    }

    pub fn speed(&self) -> f64 {
        self.speed
    }

    pub fn reset(&mut self) {
        self.start_time = 0.0;
        self.pause_offset = 0.0;
        self.playing = false;
    }
}
