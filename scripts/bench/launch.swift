import CoreGraphics
import Foundation

let bundle = CommandLine.arguments[1]
let owner = CommandLine.arguments[2]
let environment = Array(CommandLine.arguments.dropFirst(3))

let open = Process()
open.executableURL = URL(fileURLWithPath: "/usr/bin/open")
open.arguments = ["-n"] + environment.flatMap { ["--env", $0] } + [bundle]

let start = Date()
try open.run()
while Date().timeIntervalSince(start) < 60 {
    let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as? [[String: Any]] ?? []
    let shown = windows.contains { window in
        guard (window[kCGWindowOwnerName as String] as? String) == owner,
              let bounds = window[kCGWindowBounds as String] as? [String: Any],
              let width = bounds["Width"] as? Double else { return false }
        return width > 400
    }
    if shown {
        print(Int(Date().timeIntervalSince(start) * 1000))
        exit(0)
    }
    usleep(2000)
}
print("timeout")
exit(1)
