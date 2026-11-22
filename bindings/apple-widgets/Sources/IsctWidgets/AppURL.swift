import Foundation

/// The app's own URL scheme, from the running target's Info.plist
/// (`ISCT_URL_SCHEME`: the App Store app's, or the demo app's own). Links the
/// app, its widgets and its extensions make and handle all use it, so a demo
/// link never opens the App Store app.
public enum AppURL {
    public static let scheme = Bundle.main.object(forInfoDictionaryKey: "IsctURLScheme") as? String
}
