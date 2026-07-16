import OSLog

enum AppLog {
    private static let subsystem = "com.thinclaw.ios"

    static let pairing = Logger(subsystem: subsystem, category: "pairing")
    static let transport = Logger(subsystem: subsystem, category: "transport")
    static let push = Logger(subsystem: subsystem, category: "push")
    static let background = Logger(subsystem: subsystem, category: "background-refresh")
    static let snapshots = Logger(subsystem: subsystem, category: "snapshots")
    static let watchRelay = Logger(subsystem: subsystem, category: "watch-relay")
}
