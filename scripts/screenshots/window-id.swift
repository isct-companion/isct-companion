// Prints the CGWindowID of an app's main on-screen window, for
// `screencapture -l <id>` (capture.sh). Usage:
//   swift window-id.swift ["IsctCompanion Demo"]
// The owner is the app's display name; the window is the first normal-layer
// window at least 1000 points wide, so sheets, panels and menus are skipped.
import CoreGraphics
import Foundation

let owner = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "IsctCompanion Demo"
let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as? [[String: Any]] ?? []
let match = windows.first { window in
  let bounds = window[kCGWindowBounds as String] as? [String: Any] ?? [:]
  return window[kCGWindowOwnerName as String] as? String == owner
    && window[kCGWindowLayer as String] as? Int == 0
    && (bounds["Width"] as? Double ?? 0) >= 1000
}
guard let id = match?[kCGWindowNumber as String] else {
  FileHandle.standardError.write("No on-screen window of \(owner).\n".data(using: .utf8)!)
  exit(1)
}
print(id)
