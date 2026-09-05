//! The log view and command input.
//!
//! Commands go through the same dispatcher the terminal console uses, so plugin events,
//! permissions and command output behave identically whether typed here or in the terminal.

// cxx-qt expands into generated glue that does not follow the workspace's lint profile.
#![allow(
    clippy::used_underscore_binding,
    clippy::unnecessary_box_returns,
    clippy::needless_lifetimes,
    clippy::multiple_unsafe_ops_per_block,
    clippy::undocumented_unsafe_blocks
)]

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;

        include!("cxx-qt-lib/qvariant.h");
        type QVariant = cxx_qt_lib::QVariant;

        include!("cxx-qt-lib/qlist.h");
        type QList_QVariant = cxx_qt_lib::QList<QVariant>;
    }

    #[auto_cxx_name]
    extern "RustQt" {
        #[qobject]
        #[qml_element]
        /// False while the server is starting; QML disables the input until then.
        #[qproperty(bool, has_commands)]
        type Console = super::ConsoleRust;

        /// Returns log lines written since the last call, oldest first, and advances the cursor.
        ///
        /// Handing over only the delta keeps this O(new lines): re-publishing the whole buffer
        /// twice a second would copy thousands of entries for nothing.
        #[qinvokable]
        fn take_new_lines(self: Pin<&mut Self>) -> QList_QVariant;

        /// Refreshes [`Self::has_commands`]. Driven by the same timer as the rest of the UI.
        #[qinvokable]
        fn refresh(self: Pin<&mut Self>);

        /// Runs a console command.
        #[qinvokable]
        fn submit(&self, line: &QString);

        /// Tab-completion candidates for `line` at `cursor`.
        #[qinvokable]
        fn complete(&self, line: &QString, cursor: i32) -> QStringList;

        /// Begins a graceful shutdown: saves the worlds and ends the process.
        #[qinvokable]
        fn request_stop(&self);

    }
}

use core::pin::Pin;
use cxx_qt::CxxQtType;
use cxx_qt_lib::{QList, QMap, QMapPair_QString_QVariant, QString, QStringList, QVariant};

#[derive(Default)]
pub struct ConsoleRust {
    has_commands: bool,
    /// Sequence number of the next log line this view has not shown yet.
    cursor: u64,
    /// Reused between polls so the common "nothing new" case allocates nothing.
    scratch: Vec<crate::LogLine>,
}

impl qobject::Console {
    pub fn refresh(mut self: Pin<&mut Self>) {
        let has_commands = crate::gui_side().is_some_and(|side| side.commands().is_some());
        if *self.as_ref().has_commands() != has_commands {
            self.as_mut().set_has_commands(has_commands);
        }
    }

    pub fn take_new_lines(mut self: Pin<&mut Self>) -> QList<QVariant> {
        let mut out = QList::<QVariant>::default();
        let Some(side) = crate::gui_side() else {
            return out;
        };

        let cursor = self.as_ref().rust().cursor;
        let mut scratch = std::mem::take(&mut self.as_mut().rust_mut().scratch);
        scratch.clear();

        let next = side.logs.drain_since(cursor, &mut scratch);

        for line in &scratch {
            out.append(line_to_variant(line));
        }

        let mut rust = self.as_mut().rust_mut();
        rust.cursor = next;
        rust.scratch = scratch;

        out
    }

    // `&self` is required by the qinvokable signature even though the state lives in the
    // process-global GuiSide.
    #[allow(clippy::unused_self)]
    pub fn submit(&self, line: &QString) {
        let Some(side) = crate::gui_side() else {
            return;
        };
        let Some(commands) = side.commands() else {
            return;
        };

        let text = line.to_string();
        let text = text.trim().trim_start_matches('/').trim();
        if text.is_empty() {
            return;
        }

        commands.submit(text.to_owned());
    }

    #[allow(clippy::unused_self)]
    pub fn complete(&self, line: &QString, cursor: i32) -> QStringList {
        let mut list = QStringList::default();
        let Some(side) = crate::gui_side() else {
            return list;
        };
        let Some(commands) = side.commands() else {
            return list;
        };

        let text = line.to_string();
        let cursor = usize::try_from(cursor).unwrap_or(0).min(text.len());

        for candidate in commands.completions(&text, cursor) {
            list.append(QString::from(&candidate));
        }

        list
    }
}

impl qobject::Console {
    #[allow(clippy::unused_self)]
    pub fn request_stop(&self) {
        if let Some(commands) = crate::gui_side().and_then(crate::GuiSide::commands) {
            commands.request_stop();
        }
    }
}

fn line_to_variant(line: &crate::LogLine) -> QVariant {
    let mut map = QMap::<QMapPair_QString_QVariant>::default();

    map.insert(
        QString::from("level"),
        QVariant::from(&QString::from(line.level.as_str())),
    );
    map.insert(
        QString::from("target"),
        QVariant::from(&QString::from(&line.target)),
    );
    map.insert(
        QString::from("message"),
        QVariant::from(&QString::from(&line.message)),
    );

    QVariant::from(&map)
}
