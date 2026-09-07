use crate::app::{App, Mode};

pub(super) fn open(app: &mut App, raw: bool) {
    let text = if raw {
        crate::context::raw(
            &app.messages,
            app.compaction
                .as_ref()
                .map(|compact| (compact.summary.as_str(), compact.first_kept)),
        )
    } else {
        crate::context::summary(
            &app.messages,
            app.compaction
                .as_ref()
                .map(|compact| (compact.summary.as_str(), compact.first_kept)),
        )
    };
    app.mode = Mode::Context { text, scroll: 0 };
}
