import Testing

@testable import PilotageSituationCore

@Test("The XCFramework links both producer schemas")
func producerSchemasLink() {
    #expect(!ffiVersion().isEmpty)
    let versions = producerSchemaVersions()
    #expect(versions.aeroLink > 0)
    #expect(versions.surveillance > 0)
    #expect(versions.airmass > 0)
}
