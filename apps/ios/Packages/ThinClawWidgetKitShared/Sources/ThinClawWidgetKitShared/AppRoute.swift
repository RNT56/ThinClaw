import Foundation

/// Canonical route contract shared by the app, widgets, notifications, and
/// watch handoff. Parsing remains backward compatible with the legacy
/// `thinclaw://approve?request=…` widget URL.
public enum AppRoute: Sendable, Hashable {
    case pair(URL)
    case thread(String?)
    case approvals(requestID: String?, threadID: String?)
    case job(String?)
    case quickAsk

    public init?(url: URL) {
        guard url.scheme?.lowercased() == "thinclaw" else { return nil }
        let host = url.host?.lowercased()
        let components = URLComponents(url: url, resolvingAgainstBaseURL: false)
        // Treat everything after the leading slash as one opaque identifier.
        // IDs are not filesystem paths and may themselves contain `/`.
        let encodedPath = components?.percentEncodedPath.drop(while: { $0 == "/" }) ?? ""
        let pathValue =
            encodedPath.isEmpty
            ? nil
            : String(encodedPath).removingPercentEncoding ?? String(encodedPath)
        let query = components?.queryItems ?? []
        let queryValue: (String) -> String? = { name in
            query.first(where: { $0.name == name })?.value.flatMap { $0.isEmpty ? nil : $0 }
        }

        switch host {
        case "pair": self = .pair(url)
        case "thread": self = .thread(pathValue)
        case "approval":
            self = .approvals(requestID: pathValue, threadID: queryValue("thread"))
        case "approve":
            self = .approvals(
                requestID: pathValue ?? queryValue("request"),
                threadID: queryValue("thread"))
        case "approvals":
            self = .approvals(requestID: nil, threadID: queryValue("thread"))
        case "job": self = .job(pathValue)
        case "quick-ask": self = .quickAsk
        default: return nil
        }
    }

    public var url: URL {
        switch self {
        case .pair(let url): return url
        case .thread(let id): return Self.makeURL(host: "thread", path: id)
        case .approvals(let requestID, let threadID):
            return Self.makeURL(
                host: requestID == nil ? "approvals" : "approval",
                path: requestID,
                query: threadID.map { [URLQueryItem(name: "thread", value: $0)] } ?? [])
        case .job(let id): return Self.makeURL(host: "job", path: id)
        case .quickAsk: return Self.makeURL(host: "quick-ask")
        }
    }

    private static func makeURL(
        host: String,
        path: String? = nil,
        query: [URLQueryItem] = []
    ) -> URL {
        var components = URLComponents()
        components.scheme = "thinclaw"
        components.host = host
        if let path, !path.isEmpty {
            var segmentAllowed = CharacterSet.urlPathAllowed
            segmentAllowed.remove(charactersIn: "/?#")
            let encoded = path.addingPercentEncoding(withAllowedCharacters: segmentAllowed) ?? path
            components.percentEncodedPath = "/\(encoded)"
        }
        if !query.isEmpty { components.queryItems = query }
        return components.url ?? URL(string: "thinclaw://\(host)")!
    }
}
