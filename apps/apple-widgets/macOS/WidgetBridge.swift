import Foundation
import WidgetKit

@_cdecl("isct_widget_container")
public func widgetContainer(_ receive: @convention(c) (UnsafePointer<CChar>, UnsafeMutableRawPointer) -> Void,
                            _ context: UnsafeMutableRawPointer) -> Bool {
    guard let group = Bundle.main.object(forInfoDictionaryKey: "IsctWidgetGroup") as? String,
          let url = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: group)
    else { return false }
    return url.withUnsafeFileSystemRepresentation { path in
        guard let path else { return false }
        receive(path, context)
        return true
    }
}

@_cdecl("isct_widget_reload")
public func widgetReload(_ courses: Bool, _ assignments: Bool) {
    if courses { WidgetCenter.shared.reloadTimelines(ofKind: "app.isctcompanion.widgets.courses") }
    if assignments { WidgetCenter.shared.reloadTimelines(ofKind: "app.isctcompanion.widgets.assignments") }
}
