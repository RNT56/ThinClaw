import Foundation
import Testing

@testable import ThinClawAuth

@Suite("InMemoryKeychain")
struct InMemoryKeychainTests {
    @Test("set / get / remove round-trip")
    func roundTrip() throws {
        let keychain = InMemoryKeychain()
        try keychain.setSecret(Data("s3cret".utf8), for: "k")
        #expect(try keychain.secret(for: "k") == Data("s3cret".utf8))

        try keychain.removeSecret(for: "k")
        #expect(try keychain.secret(for: "k") == nil)
    }

    @Test("set replaces an existing secret")
    func setReplaces() throws {
        let keychain = InMemoryKeychain()
        try keychain.setSecret(Data("old".utf8), for: "k")
        try keychain.setSecret(Data("new".utf8), for: "k")
        #expect(try keychain.secret(for: "k") == Data("new".utf8))
        #expect(keychain.count == 1)
    }

    @Test("removing a missing key is not an error")
    func removeMissing() throws {
        try InMemoryKeychain().removeSecret(for: "never-set")
    }

    @Test("codable helpers round-trip and reject garbage")
    func codableHelpers() throws {
        let keychain = InMemoryKeychain()
        try keychain.setCodable(["a": 1, "b": 2], for: "dict")
        #expect(try keychain.codable([String: Int].self, for: "dict") == ["a": 1, "b": 2])

        try keychain.setSecret(Data("not json".utf8), for: "junk")
        #expect(throws: KeychainStoreError.invalidData) {
            _ = try keychain.codable([String: Int].self, for: "junk")
        }
    }
}

@Suite("DeviceToken")
struct DeviceTokenTests {
    @Test("well-formed tokens carry the tcd_ prefix")
    func wellFormed() {
        #expect(DeviceToken.prefix == "tcd_")
        #expect(DeviceToken.isWellFormed("tcd_8f3a9b2c4d5e"))
        #expect(!DeviceToken.isWellFormed("tcd_"))
        #expect(!DeviceToken.isWellFormed("dev_8f3a9b2c"))
        #expect(!DeviceToken.isWellFormed("tcd_has space"))
        #expect(!DeviceToken.isWellFormed(""))
    }

    @Test("redaction never leaks more than two body characters")
    func redaction() {
        #expect(DeviceToken.redacted("tcd_8f3a9b2c4d5e") == "tcd_8f…")
        #expect(DeviceToken.redacted("garbage") == "<malformed-token>")
    }
}

@Suite("DeviceCredential persistence")
struct DeviceCredentialTests {
    private var credential: DeviceCredential {
        DeviceCredential(
            installationID: "inst_9f8e",
            deviceToken: "tcd_8f3a9b2c4d5e",
            gatewayURLs: [URL(string: "https://gw.example.ts.net")!],
            serverFingerprint: "qMnE3hSuF3zXV0AJIT9cKW0eGD6dV3nCFbYbBGDs0XU",
            gatewayName: "home-server",
            pairedAt: Date(timeIntervalSince1970: 1_750_000_000))
    }

    @Test("save / load / erase round-trip through a keychain")
    func persistenceRoundTrip() throws {
        let keychain = InMemoryKeychain()
        #expect(try DeviceCredential.load(from: keychain) == nil)

        try credential.save(to: keychain)
        #expect(try DeviceCredential.load(from: keychain) == credential)

        try DeviceCredential.erase(from: keychain)
        #expect(try DeviceCredential.load(from: keychain) == nil)
    }

    @Test("credential JSON is stable across encode/decode")
    func codableRoundTrip() throws {
        let data = try JSONEncoder().encode(credential)
        let decoded = try JSONDecoder().decode(DeviceCredential.self, from: data)
        #expect(decoded == credential)
    }
}
