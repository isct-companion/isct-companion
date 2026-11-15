import Foundation
import IsctCore
import IsctWidgets
import WidgetKit

/// Core serializes this port with all publication and reset work.
final class WidgetStorage: NativeWidgets, @unchecked Sendable {
    private let directory: URL
    private let file: URL

    init?() {
        guard let group = Bundle.main.object(forInfoDictionaryKey: "IsctWidgetGroup") as? String,
              let container = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: group)
        else { return nil }
        directory = WidgetLocation.directory(in: container)
        file = WidgetLocation.file(in: container)
    }

    func current() throws -> String? {
        do {
            return try String(contentsOf: file, encoding: .utf8)
        } catch let error as CocoaError where error.code == .fileReadNoSuchFile {
            return nil
        }
    }

    func replace(json: String) throws {
        let attributes: [FileAttributeKey: Any] = [
            .protectionKey: FileProtectionType.completeUntilFirstUserAuthentication
        ]
        let manager = FileManager.default
        try manager.createDirectory(at: directory, withIntermediateDirectories: true, attributes: attributes)
        try manager.setAttributes(attributes, ofItemAtPath: directory.path)
        try Data(json.utf8).write(to: file, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
    }

    func clear() throws {
        do {
            try FileManager.default.removeItem(at: file)
        } catch let error as CocoaError where error.code == .fileNoSuchFile {}
    }

    func reload(courses: Bool, assignments: Bool) {
        if courses { WidgetCenter.shared.reloadTimelines(ofKind: WidgetPurpose.courses.kind) }
        if assignments { WidgetCenter.shared.reloadTimelines(ofKind: WidgetPurpose.assignments.kind) }
    }
}
