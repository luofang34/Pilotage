import XCTest
@testable import Pilotage

@MainActor
final class VehicleProfileTests: XCTestCase {
    func testProfileImportPersistsAndDoesNotReplaceAConflictingIdentity() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let url = directory.appendingPathComponent("vehicles.json")
        let library = VehicleProfileLibrary()
        await library.start(url: url)
        let profile = VehicleProfile(name: "Cruise", cruiseSpeedKnots: 120)
        let document = try VehicleProfileDocument.validated(PlanningCodec.encode(VehicleProfileDocument(profiles: [profile])))
        try await library.importProfiles(document)
        var conflict = profile
        conflict.cruiseSpeedKnots = 150
        do { try await library.importProfiles(VehicleProfileDocument(profiles: [conflict])); XCTFail("Conflicting import must fail") }
        catch { XCTAssertEqual(library.profiles, [profile]) }
        let reopened = VehicleProfileLibrary()
        await reopened.start(url: url)
        XCTAssertEqual(reopened.profiles, [profile])
    }

    func testSelectedProfileIsFixedUntilTheRouteUsesAnUpdatedProfile() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let plan = MissionPlanModel()
        await plan.start(url: directory.appendingPathComponent("mission.json"))
        var profile = VehicleProfile(name: "Cruise", cruiseSpeedKnots: 120)
        try await plan.vehicles.save(profile)
        plan.selectVehicle(profile)
        profile.cruiseSpeedKnots = 140
        try await plan.vehicles.save(profile)
        XCTAssertEqual(plan.route.vehicleProfile?.cruiseSpeedKnots, 120)
        let updated = try XCTUnwrap(plan.vehicles.profiles.first)
        XCTAssertEqual(updated.revision, 2)
        plan.selectVehicle(updated)
        await plan.flush()
        let reopened = MissionPlanModel()
        await reopened.start(url: directory.appendingPathComponent("mission.json"))
        XCTAssertEqual(reopened.route.vehicleProfile?.cruiseSpeedKnots, 140)
        XCTAssertNil(reopened.route.groundspeedKnots)
    }

    func testCorruptLibraryCannotBeOverwritten() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let url = directory.appendingPathComponent("vehicles.json")
        let bytes = Data("invalid".utf8)
        try bytes.write(to: url)
        let library = VehicleProfileLibrary()
        await library.start(url: url)
        do { try await library.save(VehicleProfile(name: "Cruise", cruiseSpeedKnots: 120)); XCTFail("Unreadable library must remain protected") }
        catch { XCTAssertEqual(try Data(contentsOf: url), bytes) }
    }
}
