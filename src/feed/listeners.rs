extern crate csv;
extern crate chrono;

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use config::Config;
use util::lerp;
use self::chrono::{UTC, Timelike};

const MOVING_AVG_SIZE: usize = 5;

#[derive(Debug)]
pub struct Average {
    pub current: f32,
    pub last:    f32,
    pub moving:  VecDeque<f32>,
}

impl Average {
    pub fn new(average: f32) -> Average {
        let mut moving = VecDeque::with_capacity(MOVING_AVG_SIZE + 1);
        
        if average > 0. {
            moving.push_back(average);
        }

        Average {
            current:    average,
            last:       average,
            moving:     moving,
        }
    }

    pub fn update(&mut self, value: f32) {
        self.moving.push_back(value);

        if self.moving.len() > MOVING_AVG_SIZE {
            self.moving.pop_front();
        }

        self.last    = self.current;
        self.current = self.moving.iter().sum::<f32>() / self.moving.len() as f32;
    }
}

#[derive(Debug)]
pub struct ListenerData {
    pub average:      Average,
    pub unskewed_avg: Option<f32>,
    pub hourly:       [f32; 24],
    spike_count:      u8,
}

impl ListenerData {
    pub fn new(listeners: f32, hourly: [f32; 24]) -> ListenerData {
        ListenerData {
            average:      Average::new(listeners),
            unskewed_avg: None,
            hourly:       hourly,
            spike_count:  0,
        }
    }

    pub fn step(&mut self, config: &Config, hour: usize, listeners: f32) -> bool {
        let has_spiked = self.has_spiked(&config, listeners);

        self.average.update(listeners);
        self.update_hourly(&config, hour, has_spiked);

        has_spiked
    }

    pub fn update_hourly(&mut self, config: &Config, hour: usize, has_spiked: bool) {
        self.spike_count = if has_spiked {
            self.spike_count + 1
        } else {
            0
        };

        match self.unskewed_avg {
            Some(unskewed) => {
                // Remove the unskewed average when the current average is close
                if self.average.current - unskewed < unskewed * config.unskewed_avg.reset_pcnt {
                    self.unskewed_avg = None;
                    self.hourly[hour] = self.average.current;
                } else {
                    // Slowly adjust the unskewed average to adjust to any natural listener increases
                    let new_val = lerp(unskewed,
                                        self.average.current,
                                        config.unskewed_avg.adjust_pcnt);

                    self.unskewed_avg = Some(new_val);
                    self.hourly[hour] = new_val;
                }
            },
            None => {
                if has_spiked && self.spike_count > config.unskewed_avg.spikes_required
                    && self.average.last > 0. {
                    // Use the current average instead of the last average to allow
                    // the saved average to catch up to natural listener changes
                    self.unskewed_avg = Some(self.average.current);
                }

                self.hourly[hour] = self.average.current;
            }
        }
    }

    pub fn has_spiked(&self, config: &Config, listeners: f32) -> bool {
        if self.average.current == 0. {
            return false
        }

        let spike_pcnt = config.global.spike;

        // If a feed has a low number of listeners, make the threshold higher to
        // make the calculation less sensitive to very small listener jumps
        let threshold = if listeners < 50. {
            spike_pcnt + (50. - listeners) * config.global.low_listener_increase
        } else {
            // Otherwise, decrease the threshold by a factor of how fast the feed's listeners are rising
            // to make it easier for the feed to show up in an update
            let pcnt     = config.global.high_listener_dec;
            let per_pcnt = config.global.high_listener_dec_every;

            spike_pcnt - (self.get_average_delta(listeners) / per_pcnt * pcnt).min(spike_pcnt - 0.01)
        };
        
        if cfg!(debug) {
            print!(" THR: {}", threshold);
        }
        
        (listeners - self.average.current) >= listeners * threshold
    }

    pub fn get_average_delta(&self, listeners: f32) -> f32 {
        let sub = match self.unskewed_avg {
            Some(unskewed) => unskewed,
            None => self.average.current,
        };

        listeners - sub
    }
}

pub type AverageMap = HashMap<i32, ListenerData>;

pub fn load_averages(path: &Path) -> Result<AverageMap, csv::Error> {
    let mut avgs   = HashMap::new();
    let mut reader = csv::Reader::from_file(path)?
        .has_headers(false);

    let hour = UTC::now().hour() as usize;

    for record in reader.decode() {
        let (id, avg): (_, [_; 24]) = record?;
        avgs.insert(id, ListenerData::new(avg[hour], avg));
    }

    Ok(avgs)
}

pub fn save_averages(path: &Path, averages: &AverageMap) -> Result<(), csv::Error> {
    let mut writer = csv::Writer::from_file(path)?;
    
    for (id, data) in averages {
        let hourly = data.hourly
            .iter()
            .map(|&v| v as i32)
            .collect::<Vec<_>>();

        writer.encode((id, hourly))?;
    }

    Ok(())
}