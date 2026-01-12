//
// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: MIT-0
//

//! Hold global-state of timing metrics for Application processing event and LRAP extension latency
//!
use std::time::Instant;

use once_cell::sync::OnceCell;
use parking_lot::Mutex;

static INIT_START: OnceCell<Instant> = OnceCell::new();
static APP_START: OnceCell<Instant> = OnceCell::new();

static EVENT_START: Mutex<Option<Instant>> = Mutex::new(None);

pub fn init_start() {
    if let Err(_) = INIT_START.set(Instant::now()) {
        tracing::warn!(
            "[{}] init_start() called multiple times",
            crate::log_prefix()
        );
    }
}
pub fn app_start() {
    if let Err(_) = APP_START.set(Instant::now()) {
        tracing::warn!(
            "[{}] app_start() called multiple times",
            crate::log_prefix()
        );
    }
}

#[allow(dead_code)]
pub fn get_next_event() {
    match *EVENT_START.lock() {
        None => {
            if let (Some(app_start), Some(init_start)) = (APP_START.get(), INIT_START.get()) {
                tracing::info!(
                    "[{}] Extension init     : {} us",
                    crate::log_prefix(),
                    app_start.duration_since(*init_start).as_micros()
                );
                tracing::info!(
                    "[{}] App  init     : {} us",
                    crate::log_prefix(),
                    app_start.elapsed().as_micros()
                );
            } else {
                tracing::warn!("[{}] Stats not properly initialized", crate::log_prefix());
            }
        }
        Some(event_start) => {
            tracing::info!(
                "[{}] App run time  : {} us",
                crate::log_prefix(),
                event_start.elapsed().as_micros()
            );
        }
    }
}

#[allow(dead_code)]
pub fn event_start() {
    EVENT_START.lock().replace(Instant::now());
}
