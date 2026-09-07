use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;

use crate::app::App;
use crate::mission;
use crate::protocol::{self, Usage};

pub(crate) fn start_compaction(app: &mut App, instructions: Option<&str>) {
    let Some(cfg) = app.config.clone() else {
        app.notice = Some("no model configured".into());
        return;
    };
    if app.messages.is_empty() {
        app.notice = Some("nothing to compact".into());
        return;
    }
    let boundary = app
        .compaction
        .as_ref()
        .map(|compact| compact.first_kept)
        .unwrap_or(0);
    let previous = app
        .compaction
        .as_ref()
        .map(|compact| compact.summary.as_str());
    let Some(prepared) = crate::compact::prepare(&app.messages, boundary, previous, instructions)
    else {
        app.notice = Some("nothing old enough to compact".into());
        return;
    };

    let cancel = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    app.compacting = Some(crate::app::PendingCompaction {
        first_kept: prepared.first_kept,
        tokens_before: prepared.tokens_before,
        text: String::new(),
        usage: Usage::default(),
    });
    app.cancel = Some(cancel.clone());
    app.stream_rx = Some(rx);
    app.notice = None;
    let cache_key = app.mission.as_ref().map(|mission| mission.id.clone());
    std::thread::spawn(move || {
        protocol::stream(
            cfg,
            vec![crate::protocol::ChatMessage::User(prepared.prompt)],
            cancel,
            tx,
            cache_key,
            false,
        )
    });
}

pub(crate) fn finish_compaction(app: &mut App) {
    let Some(done) = app.compacting.take() else {
        return;
    };
    app.stream_rx = None;
    app.cancel = None;
    if done.text.trim().is_empty() {
        app.notice = Some("compaction: model returned an empty summary".into());
        return;
    }
    let summary = done.text.trim().to_string();
    let tokens_after = crate::compact::estimate_context(&summary, &app.messages[done.first_kept..]);
    if done.usage.prompt() > 0 || done.usage.output > 0 {
        app.usage.add(done.usage);
        if let Some(mission) = &app.mission
            && let Err(err) = mission::append(mission, &mission::usage_line(done.usage))
        {
            app.notice = Some(format!("mission: {err}"));
            return;
        }
    }
    let Some(mission) = &app.mission else {
        app.notice = Some("compaction: no mission".into());
        return;
    };
    if let Err(err) = mission::append(
        mission,
        &mission::compaction_line(&summary, done.first_kept, done.tokens_before, tokens_after),
    ) {
        app.notice = Some(format!("mission: {err}"));
        return;
    }
    app.last_prompt = tokens_after;
    app.compaction = Some(crate::app::Compaction {
        summary,
        first_kept: done.first_kept,
    });
    app.notice = Some("context compacted".into());
}
