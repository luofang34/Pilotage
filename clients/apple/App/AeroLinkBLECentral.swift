@preconcurrency import CoreBluetooth
import Foundation
import OSLog

@MainActor
final class AeroLinkBLECentral: NSObject {
    private enum Identity {
        static let namePrefix = "Sokoly AeroLink"
        static let service = CBUUID(string: "70DCC14A-5824-4837-991A-0F7F5CD9DF2A")
        static let transmit = CBUUID(string: "93E1CF16-62EA-42DD-985A-D84490D5B4F0")
        static let legacyService = CBUUID(string: "6E400001-B5A3-F393-E0A9-E50E24DCCA9E")
        static let legacyTransmit = CBUUID(string: "6E400003-B5A3-F393-E0A9-E50E24DCCA9E")
        static let services = [service, legacyService]

        static func transmit(for service: CBUUID) -> CBUUID? {
            switch service {
            case Self.service: Self.transmit
            case Self.legacyService: Self.legacyTransmit
            default: nil
            }
        }
    }

    private let publish: (AeroLinkBLEEvent) -> Void
    private let logger: Logger
    private var central: CBCentralManager?
    private var peripheral: CBPeripheral?
    private var connection: AeroLinkBLEConnection?
    private var active = false
    private var reconnectGeneration: UInt64 = 0

    init(publish: @escaping (AeroLinkBLEEvent) -> Void) {
        self.publish = publish
        logger = Logger(
            subsystem: Bundle.main.bundleIdentifier ?? "org.luofang.pilotage",
            category: "aerolink-appliance"
        )
        super.init()
    }

    func start() {
        guard !active else { return }
        active = true
        publish(.state(.checking, nil))
        if central == nil {
            central = CBCentralManager(
                delegate: self,
                queue: .main,
                options: [CBCentralManagerOptionShowPowerAlertKey: true]
            )
        } else {
            scanIfReady()
        }
    }

    func stop() {
        active = false
        central?.stopScan()
        if let peripheral {
            central?.cancelPeripheralConnection(peripheral)
        }
        self.peripheral = nil
        connection = nil
        publish(.state(.off, nil))
    }

    private func scanIfReady() {
        guard active, central?.state == .poweredOn, peripheral == nil else { return }
        publish(.state(.checking, nil))
        central?.scanForPeripherals(
            withServices: Identity.services,
            options: [CBCentralManagerScanOptionAllowDuplicatesKey: false]
        )
    }

    private func connect(_ peripheral: CBPeripheral, name: String) {
        central?.stopScan()
        reconnectGeneration &+= 1
        let value = AeroLinkBLEConnection(
            name: name,
            identifier: peripheral.identifier.uuidString,
            sourceId: Self.sourceId(for: peripheral.identifier),
            reconnectGeneration: reconnectGeneration
        )
        self.peripheral = peripheral
        connection = value
        peripheral.delegate = self
        publish(.state(.connecting, value))
        central?.connect(peripheral)
    }

    private func disconnect(_ detail: String?) {
        if let detail {
            logger.error("BLE connection stopped: \(detail, privacy: .public)")
            publish(.state(.unavailable(detail), connection))
        }
        peripheral = nil
        connection = nil
        scanIfReady()
    }

    private static func sourceId(for identifier: UUID) -> UInt32 {
        var value: UInt32 = 2_166_136_261
        for byte in identifier.uuidString.utf8 {
            value = (value ^ UInt32(byte)) &* 16_777_619
        }
        return value == 0 ? 1 : value
    }
}

extension AeroLinkBLECentral: @preconcurrency CBCentralManagerDelegate {
    func centralManagerDidUpdateState(_ central: CBCentralManager) {
        guard active else { return }
        switch central.state {
        case .poweredOn:
            scanIfReady()
        case .poweredOff:
            publish(.state(.unavailable("Bluetooth is off."), nil))
        case .unauthorized:
            publish(.state(.unavailable("Pilotage does not have Bluetooth access."), nil))
        case .unsupported:
            publish(.state(.unavailable("This device does not support Bluetooth LE."), nil))
        case .resetting, .unknown:
            publish(.state(.checking, nil))
        @unknown default:
            publish(.state(.unavailable("Bluetooth is not available."), nil))
        }
    }

    func centralManager(
        _ central: CBCentralManager,
        didDiscover peripheral: CBPeripheral,
        advertisementData: [String: Any],
        rssi: NSNumber
    ) {
        guard active, self.peripheral == nil else { return }
        let advertisedName = advertisementData[CBAdvertisementDataLocalNameKey] as? String
        let name = advertisedName ?? peripheral.name ?? ""
        guard name.hasPrefix(Identity.namePrefix) else { return }
        connect(peripheral, name: name)
    }

    func centralManager(_ central: CBCentralManager, didConnect peripheral: CBPeripheral) {
        publish(.state(.connecting, connection))
        peripheral.discoverServices(Identity.services)
    }

    func centralManager(
        _ central: CBCentralManager,
        didFailToConnect peripheral: CBPeripheral,
        error: (any Error)?
    ) {
        disconnect(error?.localizedDescription ?? "The AeroLink connection failed.")
    }

    func centralManager(
        _ central: CBCentralManager,
        didDisconnectPeripheral peripheral: CBPeripheral,
        timestamp: CFAbsoluteTime,
        isReconnecting: Bool,
        error: (any Error)?
    ) {
        disconnect(error?.localizedDescription)
    }
}

extension AeroLinkBLECentral: @preconcurrency CBPeripheralDelegate {
    func peripheral(_ peripheral: CBPeripheral, didDiscoverServices error: (any Error)?) {
        if let error {
            disconnect(error.localizedDescription)
            return
        }
        for service in peripheral.services ?? [] {
            guard let transmit = Identity.transmit(for: service.uuid) else { continue }
            peripheral.discoverCharacteristics([transmit], for: service)
        }
    }

    func peripheral(
        _ peripheral: CBPeripheral,
        didDiscoverCharacteristicsFor service: CBService,
        error: (any Error)?
    ) {
        if let error {
            disconnect(error.localizedDescription)
            return
        }
        guard let expected = Identity.transmit(for: service.uuid),
              let characteristic = service.characteristics?.first(where: {
                  $0.uuid == expected && $0.properties.contains(.notify)
              }) else { return }
        peripheral.setNotifyValue(true, for: characteristic)
    }

    func peripheral(
        _ peripheral: CBPeripheral,
        didUpdateNotificationStateFor characteristic: CBCharacteristic,
        error: (any Error)?
    ) {
        if let error {
            disconnect(error.localizedDescription)
            return
        }
        guard characteristic.isNotifying else { return }
        publish(.state(.ready, connection))
    }

    func peripheral(
        _ peripheral: CBPeripheral,
        didUpdateValueFor characteristic: CBCharacteristic,
        error: (any Error)?
    ) {
        if let error {
            logger.error("BLE notification failed: \(error.localizedDescription, privacy: .public)")
            return
        }
        guard let data = characteristic.value, !data.isEmpty, let connection else { return }
        publish(.bytes(data, connection))
    }
}
