import Foundation

#if DEBUG
/// Prints the process physical footprint once per interval, so a
/// console-attached run shows a leak as a slope, not a surprise.
enum FootprintProbe {
    static func start() {
        guard LaunchRequest.openInstruments else { return }
        Timer.scheduledTimer(withTimeInterval: 10, repeats: true) { _ in
            var info = task_vm_info_data_t()
            var count = mach_msg_type_number_t(
                MemoryLayout<task_vm_info_data_t>.size / MemoryLayout<integer_t>.size)
            let result = withUnsafeMutablePointer(to: &info) {
                $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
                    task_info(mach_task_self_, task_flavor_t(TASK_VM_INFO), $0, &count)
                }
            }
            if result == KERN_SUCCESS {
                let mb = Double(info.phys_footprint) / 1_048_576
                print("harness memory: footprint \(Int(mb)) MB")
            }
        }
    }
}
#endif
