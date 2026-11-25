use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Weak};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui;
use serde::Deserialize;

use super::{CLIENT_ID, post, save_token};

const DEVICE_CODE: &str = "https://github.com/login/device/code";

const ACCESS_TOKEN: &str = "https://github.com/login/oauth/access_token";

const GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

const SLOW_DOWN: u64 = 5;

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Code { code: String, url: String },
    Done(Result<(), String>),
}

pub struct Login {
    pub code: Option<(String, String)>,
    events: Receiver<Event>,
    _alive: Arc<()>,
}

impl Login {
    pub fn start(ctx: egui::Context) -> Self {
        let (sender, events) = mpsc::channel();
        let alive = Arc::new(());
        let watched = Arc::downgrade(&alive);
        thread::spawn(move || {
            let result = log_in(&sender, &watched, &ctx);
            let _ = sender.send(Event::Done(result));
            ctx.request_repaint();
        });
        Self {
            code: None,
            events,
            _alive: alive,
        }
    }

    pub fn next(&mut self) -> Option<Event> {
        let event = self.events.try_recv().ok()?;
        if let Event::Code { code, url } = &event {
            self.code = Some((code.clone(), url.clone()));
        }
        Some(event)
    }
}

#[derive(Deserialize)]
struct Device {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
}

#[derive(Deserialize, Default)]
struct Answer {
    access_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
    interval: Option<u64>,
}

#[derive(Debug, PartialEq)]
enum Poll {
    Token(String),
    Wait(Option<u64>),
    Fail(String),
}

fn log_in(sender: &Sender<Event>, alive: &Weak<()>, ctx: &egui::Context) -> Result<(), String> {
    if CLIENT_ID.is_empty() {
        return Err("This Shepherd has no GitHub app to log in with".to_string());
    }
    let response = post(
        DEVICE_CODE,
        &[("client_id", CLIENT_ID), ("scope", "repo")],
        None,
    )?;
    let device: Device = serde_json::from_str(&response.body)
        .map_err(|_| format!("GitHub did not give a login code: {}", response.body))?;
    let _ = sender.send(Event::Code {
        code: device.user_code,
        url: device.verification_uri,
    });
    ctx.request_repaint();
    let deadline = Instant::now() + Duration::from_secs(device.expires_in);
    let mut interval = device.interval;
    loop {
        thread::sleep(Duration::from_secs(interval));
        if alive.strong_count() == 0 {
            return Err("The login was cancelled".to_string());
        }
        if Instant::now() >= deadline {
            return Err("The login code expired. Log in again.".to_string());
        }
        let response = post(
            ACCESS_TOKEN,
            &[("client_id", CLIENT_ID), ("grant_type", GRANT)],
            Some(("device_code", &device.device_code)),
        )?;
        match poll(&response.body) {
            Poll::Token(token) => return save_token(&token),
            Poll::Wait(slower) => interval = slower.unwrap_or(interval),
            Poll::Fail(error) => return Err(error),
        }
    }
}

fn poll(body: &str) -> Poll {
    let answer: Answer = serde_json::from_str(body).unwrap_or_default();
    if let Some(token) = answer.access_token {
        return Poll::Token(token);
    }
    match answer.error.as_deref() {
        Some("authorization_pending") => Poll::Wait(None),
        Some("slow_down") => Poll::Wait(Some(answer.interval.unwrap_or(SLOW_DOWN * 2))),
        Some("expired_token") => Poll::Fail("The login code expired. Log in again.".to_string()),
        Some("access_denied") => Poll::Fail("The login was turned down on GitHub".to_string()),
        _ => Poll::Fail(
            answer
                .error_description
                .unwrap_or_else(|| "GitHub did not let Shepherd log in".to_string()),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_until_the_code_is_entered_and_then_takes_the_token() {
        assert_eq!(
            poll(r#"{"error":"authorization_pending"}"#),
            Poll::Wait(None)
        );
        assert_eq!(
            poll(r#"{"error":"slow_down","interval":10}"#),
            Poll::Wait(Some(10))
        );
        assert_eq!(
            poll(r#"{"access_token":"gho_abc","token_type":"bearer","scope":"repo"}"#),
            Poll::Token("gho_abc".to_string())
        );
    }

    #[test]
    fn stops_when_the_code_expires_or_is_turned_down() {
        assert_eq!(
            poll(r#"{"error":"expired_token"}"#),
            Poll::Fail("The login code expired. Log in again.".to_string())
        );
        assert_eq!(
            poll(r#"{"error":"access_denied"}"#),
            Poll::Fail("The login was turned down on GitHub".to_string())
        );
        assert_eq!(
            poll(r#"{"error":"device_flow_disabled","error_description":"Device flow is off"}"#),
            Poll::Fail("Device flow is off".to_string())
        );
        assert!(matches!(poll("not json"), Poll::Fail(_)));
    }
}
