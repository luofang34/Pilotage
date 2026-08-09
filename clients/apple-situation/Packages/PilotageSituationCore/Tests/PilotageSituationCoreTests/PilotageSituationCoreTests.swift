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

@Test("Layer controls cross the generated facade")
func layerControlsCrossFacade() throws {
    let session = PresentationSession()
    let batch = try session.observeSources(
        observation: PresentationSourceObservation(
            terrainAvailable: true,
            radioState: .streaming,
            radioReceivers: [
                PresentationReceiverObservation(band: .adsb1090, state: .streaming),
            ]
        ),
        nowMicros: 10
    )

    let allEnabled = batch.layers.allSatisfy(\.enabled)
    #expect(batch.layers.count == 4)
    #expect(allEnabled)
    #expect(batch.layers.first { $0.id == "traffic" }?.sourceState == .live)
    #expect(
        batch.layers.first { $0.id == "weather-reports" }?
            .sourceDetail.contains("does not mean clear weather") == true
    )
}
