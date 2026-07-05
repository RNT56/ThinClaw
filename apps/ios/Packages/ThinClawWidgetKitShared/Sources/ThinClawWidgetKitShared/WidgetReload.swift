#if canImport(WidgetKit)
    import WidgetKit

    /// Timeline-reload helpers for the interactive intents.
    ///
    /// After an intent mutates shared state (an approval decision, a Quick Ask
    /// receipt) it asks WidgetKit to rebuild the affected timelines so the row
    /// disappears / the "sent" state appears without waiting for the next
    /// scheduled refresh. Widget kinds are kept in sync with each widget's
    /// `StaticConfiguration(kind:)`.
    public enum WidgetReload {
        /// Kind strings, mirrored from the widget `StaticConfiguration`s.
        public enum Kind {
            public static let status = "com.thinclaw.ios.widget.status"
            public static let approvals = "com.thinclaw.ios.widget.approvals"
            public static let quickAsk = "com.thinclaw.ios.widget.quickask"
        }

        /// Reload the approvals widget (a decision removes a row) and the
        /// status widget (its pending count changed).
        public static func approvals() {
            WidgetCenter.shared.reloadTimelines(ofKind: Kind.approvals)
            WidgetCenter.shared.reloadTimelines(ofKind: Kind.status)
        }

        /// Reload the Quick Ask widget after writing a send receipt.
        public static func quickAsk() {
            WidgetCenter.shared.reloadTimelines(ofKind: Kind.quickAsk)
        }
    }
#endif
