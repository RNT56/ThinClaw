import Foundation
import Testing

@testable import ThinClawAuth

/// Builds a `thinclaw://pair?d=…` URL from raw JSON, exactly the way the
/// gateway's QR generator does.
private func pairingURL(json: String) -> URL {
    let encoded = Data(json.utf8).base64URLEncodedString()
    return URL(string: "thinclaw://pair?d=\(encoded)")!
}

private let farFuture = 4_102_444_800  // 2100-01-01
private let now = Date(timeIntervalSince1970: 1_750_000_000)

private func validJSON(
    v: Int = 1,
    urls: String = #"["https://gw.example.ts.net","http://192.168.1.20:3000"]"#,
    exp: Int = farFuture,
    extra: String = ""
) -> String {
    """
    {"v":\(v),"urls":\(urls),"fp":"sha256:ab12cd34","iid":"inst_9f8e",\
    "name":"home-server","sec":"pair_5f4dcc3b5aa7","exp":\(exp)\(extra)}
    """
}

@Suite("PairingPayload parsing")
struct PairingPayloadParsingTests {
    @Test("a valid v1 payload parses completely")
    func validPayload() throws {
        let payload = try PairingPayload.parse(from: pairingURL(json: validJSON()), now: now)

        #expect(payload.version == 1)
        #expect(
            payload.urls == [
                URL(string: "https://gw.example.ts.net")!,
                URL(string: "http://192.168.1.20:3000")!,
            ])
        #expect(payload.fingerprint == "sha256:ab12cd34")
        #expect(payload.installationID == "inst_9f8e")
        #expect(payload.name == "home-server")
        #expect(payload.secret == "pair_5f4dcc3b5aa7")
        #expect(payload.expiresAt == Date(timeIntervalSince1970: TimeInterval(farFuture)))
    }

    @Test("fp is optional")
    func optionalFingerprint() throws {
        let json = """
            {"v":1,"urls":["https://gw.local"],"iid":"i","name":"n","sec":"s","exp":\(farFuture)}
            """
        let payload = try PairingPayload.parse(from: pairingURL(json: json), now: now)
        #expect(payload.fingerprint == nil)
    }

    @Test("unknown extra fields are ignored (forward compatible)")
    func extraFieldsIgnored() throws {
        let json = validJSON(extra: #","future_field":{"nested":true},"hint":42"#)
        let payload = try PairingPayload.parse(from: pairingURL(json: json), now: now)
        #expect(payload.installationID == "inst_9f8e")
    }

    @Test("base64url without padding decodes (the gateway never pads)")
    func base64URLNoPadding() throws {
        // validJSON length varies; assert the helper itself never emits '='.
        let encoded = Data(validJSON().utf8).base64URLEncodedString()
        #expect(!encoded.contains("="))
        _ = try PairingPayload.parse(from: pairingURL(json: validJSON()), now: now)
    }
}

@Suite("PairingPayload rejection")
struct PairingPayloadRejectionTests {
    @Test("unknown version is rejected")
    func unknownVersion() {
        #expect(throws: PairingPayloadError.unsupportedVersion(2)) {
            _ = try PairingPayload.parse(from: pairingURL(json: validJSON(v: 2)), now: now)
        }
    }

    @Test("expired payload is rejected")
    func expired() {
        let exp = Int(now.timeIntervalSince1970) - 60
        #expect(throws: PairingPayloadError.expired(Date(timeIntervalSince1970: TimeInterval(exp)))) {
            _ = try PairingPayload.parse(from: pairingURL(json: validJSON(exp: exp)), now: now)
        }
    }

    @Test("payload expiring exactly now is rejected (strict >)")
    func expiryBoundary() {
        let exp = Int(now.timeIntervalSince1970)
        #expect(throws: PairingPayloadError.self) {
            _ = try PairingPayload.parse(from: pairingURL(json: validJSON(exp: exp)), now: now)
        }
    }

    @Test("empty urls array is rejected")
    func emptyURLs() {
        #expect(throws: PairingPayloadError.noUsableURLs) {
            _ = try PairingPayload.parse(from: pairingURL(json: validJSON(urls: "[]")), now: now)
        }
    }

    @Test("urls with only junk entries are rejected; junk among good is filtered")
    func junkURLs() throws {
        #expect(throws: PairingPayloadError.noUsableURLs) {
            _ = try PairingPayload.parse(
                from: pairingURL(json: validJSON(urls: #"["ftp://x","not a url",""]"#)),
                now: now)
        }

        let mixed = try PairingPayload.parse(
            from: pairingURL(json: validJSON(urls: #"["ftp://x","https://good.example"]"#)),
            now: now)
        #expect(mixed.urls == [URL(string: "https://good.example")!])
    }

    @Test("wrong scheme, wrong host, and missing d are not pairing URLs")
    func notPairingURLs() {
        for raw in [
            "https://pair?d=abc",
            "thinclaw://settings?d=abc",
            "thinclaw://pair",
            "thinclaw://pair?d=",
        ] {
            #expect(throws: PairingPayloadError.notAPairingURL, "\(raw)") {
                _ = try PairingPayload.parse(from: URL(string: raw)!, now: now)
            }
        }
    }

    @Test("undecodable base64 and non-JSON payloads are malformed")
    func malformedPayloads() {
        for d in ["%%%%", "!!!", Data("not json".utf8).base64URLEncodedString()] {
            #expect(throws: PairingPayloadError.malformedPayload, "d=\(d)") {
                _ = try PairingPayload.parse(
                    from: URL(string: "thinclaw://pair?d=\(d)")!, now: now)
            }
        }
    }

    @Test("JSON missing required fields is malformed")
    func missingRequiredFields() {
        let json = #"{"v":1,"urls":["https://gw.local"]}"#
        #expect(throws: PairingPayloadError.malformedPayload) {
            _ = try PairingPayload.parse(from: pairingURL(json: json), now: now)
        }
    }
}

@Suite("base64url codec")
struct Base64URLTests {
    @Test("round-trips arbitrary bytes at every padding length")
    func roundTrip() throws {
        for length in 0..<16 {
            let bytes = Data((0..<length).map { UInt8(truncatingIfNeeded: $0 &* 37 &+ 11) })
            let encoded = bytes.base64URLEncodedString()
            #expect(!encoded.contains("="))
            #expect(!encoded.contains("+"))
            #expect(!encoded.contains("/"))
            #expect(Data(base64URLEncoded: encoded) == bytes)
        }
    }

    @Test("accepts padded input too")
    func paddedInput() {
        #expect(Data(base64URLEncoded: "aGk=") == Data("hi".utf8))
    }

    @Test("rejects impossible lengths")
    func impossibleLength() {
        #expect(Data(base64URLEncoded: "a") == nil)
    }
}
