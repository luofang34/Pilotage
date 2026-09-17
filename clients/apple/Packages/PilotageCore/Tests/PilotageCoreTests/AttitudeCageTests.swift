import Testing
@testable import PilotageCore

@Test func cageMakesTheCapturedPitchAndBankLevel() {
    var cage = AttitudeCage()
    let accepted = cage.recenter(on: AircraftAttitude(rollDegrees: 4.5, pitchDegrees: -2))
    #expect(accepted)
    #expect(
        cage.apply(to: AircraftAttitude(rollDegrees: 4.5, pitchDegrees: -2))
            == AircraftAttitude(rollDegrees: 0, pitchDegrees: 0)
    )
}

@Test func cageLeavesLaterMotionRelativeToTheCapturedLevel() {
    let cage = AttitudeCage(rollOffsetDegrees: 4.5, pitchOffsetDegrees: -2)
    #expect(
        cage.apply(to: AircraftAttitude(rollDegrees: 14.5, pitchDegrees: 3))
            == AircraftAttitude(rollDegrees: 10, pitchDegrees: 5)
    )
}

@Test func cageDoesNotAcceptAnInvalidAttitude() {
    var cage = AttitudeCage()
    let accepted = cage.recenter(on: AircraftAttitude(rollDegrees: .nan, pitchDegrees: 0))
    #expect(!accepted)
    #expect(cage.rollOffsetDegrees == nil)
    #expect(cage.pitchOffsetDegrees == nil)
}
