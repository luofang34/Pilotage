import PilotageRadioSource
import PilotageSituationCore

extension PresentationSourceObservation {
    init(source: RadioSourceSnapshot, terrainAvailable: Bool) {
        self.init(
            terrainAvailable: terrainAvailable,
            radioState: PresentationRadioState(source.availability),
            radioReceivers: source.receivers.map(PresentationReceiverObservation.init)
        )
    }
}

private extension PresentationReceiverObservation {
    init(_ receiver: RadioReceiver) {
        self.init(
            band: PresentationRadioBand(receiver.band),
            state: PresentationRadioState(receiver.availability)
        )
    }
}

private extension PresentationRadioBand {
    init(_ band: RadioBand) {
        switch band {
        case .adsb1090: self = .adsb1090
        case .uat978: self = .uat978
        }
    }
}

private extension PresentationRadioState {
    init(_ availability: RadioAvailability) {
        switch availability {
        case .checking: self = .checking
        case .permissionDenied: self = .permissionDenied
        case .driverDisabled: self = .driverDisabled
        case .unplugged: self = .unplugged
        case .ready: self = .ready
        case .streaming: self = .streaming
        case .suspended: self = .suspended
        case .underpowered: self = .underpowered
        case .enumerationFailure: self = .enumerationFailure
        case .endpointFailure: self = .endpointFailure
        case .deviceRemoved: self = .deviceRemoved
        }
    }
}
