import Foundation
import Testing

@testable import ThinClawAuth

// MARK: - TXT-record parsing

@Suite("Discovery TXT-record parsing")
struct DiscoveryTXTRecordTests {
    @Test("a full record parses every locator hint")
    func fullRecord() {
        let txt = DiscoveryTXTRecord(dictionary: [
            "version": "0.9.0",
            "api": "v1",
            "name": "ThinClaw on mac-mini",
            "fp": "qMnE3hSuF3zXV0AJIT9cKW0eGD6dV3nCFbYbBGDs0XU",
        ])
        #expect(txt.version == "0.9.0")
        #expect(txt.apiVersion == "v1")
        #expect(txt.name == "ThinClaw on mac-mini")
        #expect(txt.instanceFingerprint == "qMnE3hSuF3zXV0AJIT9cKW0eGD6dV3nCFbYbBGDs0XU")
    }

    @Test("an unpaired gateway advertises no fingerprint")
    func noFingerprint() {
        let txt = DiscoveryTXTRecord(dictionary: [
            "version": "0.9.0", "api": "v1", "name": "fresh-gateway",
        ])
        #expect(txt.instanceFingerprint == nil)
        #expect(txt.name == "fresh-gateway")
    }

    @Test("empty and whitespace-only values are treated as absent")
    func emptyValuesDropped() {
        let txt = DiscoveryTXTRecord(dictionary: [
            "version": "", "api": "   ", "name": "", "fp": "",
        ])
        #expect(txt.version == nil)
        #expect(txt.apiVersion == nil)
        #expect(txt.name == nil)
        #expect(txt.instanceFingerprint == nil)
    }

    @Test("unknown keys are ignored (forward compatible)")
    func unknownKeysIgnored() {
        let txt = DiscoveryTXTRecord(dictionary: [
            "api": "v2", "future_flag": "1", "name": "next",
        ])
        #expect(txt.apiVersion == "v2")
        #expect(txt.name == "next")
        // No crash / no leakage: only declared keys surface.
        #expect(txt.instanceFingerprint == nil)
    }

    @Test("an empty record parses to all-nil hints")
    func emptyRecord() {
        let txt = DiscoveryTXTRecord(dictionary: [:])
        #expect(txt == DiscoveryTXTRecord())
    }
}

// MARK: - Instance fingerprint (D-X3 locator match)

@Suite("Discovery instance fingerprint")
struct DiscoveryFingerprintTests {
    // Mirrors the server's `fingerprint_instance_id`: unpadded base64url of
    // sha256(instance-id). Precomputed for a fixed id so a server/client drift
    // would fail this test.
    private let instanceID = "11111111-2222-3333-4444-555555555555"

    @Test("fingerprint is deterministic, 43-char unpadded base64url")
    func fingerprintShape() {
        let fp = DiscoveryTXTRecord.fingerprint(ofInstanceID: instanceID)
        #expect(fp == DiscoveryTXTRecord.fingerprint(ofInstanceID: instanceID))
        #expect(fp.count == 43)  // sha256 digest, base64url, unpadded
        #expect(!fp.contains("="))
        #expect(fp.allSatisfy { $0.isLetter || $0.isNumber || $0 == "-" || $0 == "_" })
    }

    @Test("fingerprint never embeds the raw instance id")
    func fingerprintNotReversible() {
        let fp = DiscoveryTXTRecord.fingerprint(ofInstanceID: instanceID)
        #expect(!fp.contains(instanceID))
    }

    @Test("matchesInstance is true only for the matching id")
    func matchesOnlyCorrectID() {
        let fp = DiscoveryTXTRecord.fingerprint(ofInstanceID: instanceID)
        let record = DiscoveryTXTRecord(instanceFingerprint: fp)
        #expect(record.matchesInstance(id: instanceID))
        #expect(!record.matchesInstance(id: "different-instance"))
    }

    @Test("a record with no fingerprint never matches (locator, not authenticator)")
    func noFingerprintNeverMatches() {
        let record = DiscoveryTXTRecord(instanceFingerprint: nil)
        #expect(!record.matchesInstance(id: instanceID))
    }
}

// MARK: - DiscoveredGateway model

@Suite("DiscoveredGateway model")
struct DiscoveredGatewayTests {
    @Test("displayName prefers the TXT name over the Bonjour instance name")
    func displayNamePrefersTXT() {
        let gateway = DiscoveredGateway(
            name: "thinclaw-abc123",
            txt: DiscoveryTXTRecord(name: "ThinClaw on mac-mini"))
        #expect(gateway.displayName == "ThinClaw on mac-mini")
    }

    @Test("displayName falls back to the Bonjour instance name")
    func displayNameFallback() {
        let gateway = DiscoveredGateway(name: "thinclaw-abc123")
        #expect(gateway.displayName == "thinclaw-abc123")
    }

    @Test("suggestedBaseURL is nil until host and port resolve")
    func suggestedURLNilUnresolved() {
        let unresolved = DiscoveredGateway(name: "gw")
        #expect(unresolved.suggestedBaseURL == nil)
        let hostOnly = DiscoveredGateway(name: "gw", host: "192.168.1.5")
        #expect(hostOnly.suggestedBaseURL == nil)
    }

    @Test("suggestedBaseURL is an https URL once resolved")
    func suggestedURLResolved() {
        let gateway = DiscoveredGateway(name: "gw", host: "192.168.1.5", port: 3443)
        #expect(gateway.suggestedBaseURL == URL(string: "https://192.168.1.5:3443"))
    }

    @Test("suggestedBaseURL brackets IPv6 literals and strips a trailing dot")
    func suggestedURLIPv6() {
        #expect(
            DiscoveredGateway.suggestedBaseURL(host: "fe80::1", port: 3443)
                == URL(string: "https://[fe80::1]:3443"))
        #expect(
            DiscoveredGateway.suggestedBaseURL(host: "mac-mini.local.", port: 3443)
                == URL(string: "https://mac-mini.local:3443"))
    }

    @Test("instanceFingerprint surfaces the TXT fp hint")
    func fingerprintPassThrough() {
        let gateway = DiscoveredGateway(
            name: "gw", txt: DiscoveryTXTRecord(instanceFingerprint: "abc"))
        #expect(gateway.instanceFingerprint == "abc")
    }
}
