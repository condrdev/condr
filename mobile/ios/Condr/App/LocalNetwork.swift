import Darwin

/// iOS asks for Local Network access the first time the app reaches its network, and may
/// refuse the very operation that raised the prompt (TN3179). That must not be the pairing
/// dial, so the app raises the prompt itself as pairing starts (ADR 0040): connecting a UDP
/// socket to another host on a link-local network sends nothing and still asks. Best
/// effort, as TN3179's own version is.
enum LocalNetwork {
    static func ask() {
        var list: UnsafeMutablePointer<ifaddrs>?
        guard getifaddrs(&list) == 0, let first = list else { return }
        defer { freeifaddrs(first) }
        for entry in sequence(first: first, next: { $0.pointee.ifa_next }) {
            guard entry.pointee.ifa_flags & UInt32(IFF_BROADCAST) != 0,
                let address = entry.pointee.ifa_addr,
                address.pointee.sa_family == AF_INET6,
                Int(address.pointee.sa_len) >= MemoryLayout<sockaddr_in6>.size
            else { continue }
            var target = UnsafeRawPointer(address).load(as: sockaddr_in6.self)
            // fe80::/10: the interface's link-local network.
            let prefix = target.sin6_addr.__u6_addr.__u6_addr8
            guard prefix.0 == 0xfe, prefix.1 & 0xc0 == 0x80 else { continue }
            // Another host on that link, at the discard port.
            withUnsafeMutableBytes(of: &target.sin6_addr) { bytes in
                for index in 8..<16 { bytes[index] = .random(in: 0...255) }
            }
            target.sin6_port = UInt16(9).bigEndian
            let socket = Darwin.socket(AF_INET6, SOCK_DGRAM, 0)
            guard socket >= 0 else { continue }
            defer { close(socket) }
            withUnsafePointer(to: &target) {
                $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                    _ = connect(socket, $0, socklen_t(MemoryLayout<sockaddr_in6>.size))
                }
            }
        }
    }
}
