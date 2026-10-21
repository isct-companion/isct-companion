//! macOS notifications through UNUserNotificationCenter.
//!
//! The signed app talks to UserNotifications directly: it asks for
//! authorization with the system prompt, reads the real status, and requests
//! banners while the app is in front.

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{Bool, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::NSApplication;
use objc2_foundation::{NSBundle, NSError, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNAuthorizationStatus, UNMutableNotificationContent, UNNotification,
    UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationResponse,
    UNNotificationSettings, UNNotificationSound, UNUserNotificationCenter,
    UNUserNotificationCenterDelegate,
};
use std::ptr::NonNull;
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    struct ForegroundBanners;

    unsafe impl NSObjectProtocol for ForegroundBanners {}

    unsafe impl UNUserNotificationCenterDelegate for ForegroundBanners {
        // Without this the system drops banners while IsctCompanion is the
        // active app, which is exactly when a class reminder matters.
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            handler: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            handler.call((UNNotificationPresentationOptions::Banner
                | UNNotificationPresentationOptions::List
                | UNNotificationPresentationOptions::Sound,));
        }

        // Clicking a banner should bring the app back even when its window was
        // behind another app. The response handler must always be completed so
        // macOS can finish processing the notification action.
        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive_response(
            &self,
            _center: &UNUserNotificationCenter,
            _response: &UNNotificationResponse,
            handler: &block2::DynBlock<dyn Fn()>,
        ) {
            NSApplication::sharedApplication(self.mtm()).activate();
            handler.call(());
        }
    }
);

pub(crate) const UNBUNDLED: &str =
    "Open the installed IsctCompanion.app to enable notifications. This development process cannot request macOS notification permission.";

pub(crate) fn available() -> bool {
    NSBundle::mainBundle()
        .bundlePath()
        .to_string()
        .ends_with(".app")
}

/// The notification center, or None when the process was not launched from an
/// app bundle (the bare `tauri dev` binary). There `currentNotificationCenter`
/// raises an Objective-C exception, which unwinds into Rust and kills the
/// process, so every call goes through here.
fn center() -> Option<Retained<UNUserNotificationCenter>> {
    available().then(UNUserNotificationCenter::currentNotificationCenter)
}

/// Install the foreground presenter once, on the main thread at startup. The
/// center holds its delegate weakly, so the app keeps it for its lifetime.
pub(crate) fn install(main: MainThreadMarker) {
    let Some(center) = center() else { return };
    let delegate: Retained<ForegroundBanners> =
        unsafe { msg_send![ForegroundBanners::alloc(main), init] };
    center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    std::mem::forget(delegate);
}

fn once<T: Send + 'static>() -> (impl Fn(T) + Clone, oneshot::Receiver<T>) {
    let (tx, rx) = oneshot::channel();
    let slot = Arc::new(Mutex::new(Some(tx)));
    let send = move |value: T| {
        if let Some(sender) = slot.lock().ok().and_then(|mut slot| slot.take()) {
            let _ = sender.send(value);
        }
    };
    (send, rx)
}

unsafe fn error_text(error: *mut NSError) -> Option<String> {
    error
        .as_ref()
        .map(|error| error.localizedDescription().to_string())
}

// Each request registers its callback in a plain function so no Objective-C
// object is held across an await: they are not Send.

/// The real authorization status: `granted`, `denied` or `default` (never asked).
pub(crate) async fn permission() -> Result<&'static str, String> {
    permission_request()
        .await
        .map_err(|_| "notification permission could not be read".to_string())?
}

fn permission_request() -> oneshot::Receiver<Result<&'static str, String>> {
    let (send, rx) = once::<Result<&'static str, String>>();
    let Some(center) = center() else {
        send(Err(UNBUNDLED.into()));
        return rx;
    };
    let callback = RcBlock::new(move |settings: NonNull<UNNotificationSettings>| {
        let status = unsafe { settings.as_ref() }.authorizationStatus();
        send(Ok(match status {
            UNAuthorizationStatus::Authorized
            | UNAuthorizationStatus::Provisional
            | UNAuthorizationStatus::Ephemeral => "granted",
            UNAuthorizationStatus::Denied => "denied",
            _ => "default",
        }));
    });
    center.getNotificationSettingsWithCompletionHandler(&callback);
    rx
}

/// Shows the system prompt the first time; afterwards macOS answers from the
/// user's choice in System Settings without asking again. Only a real Denied
/// status is a refusal; an unbundled process or an API error is not permission.
pub(crate) async fn request_permission() -> Result<&'static str, String> {
    match permission().await? {
        "granted" => return Ok("granted"),
        "denied" => return Ok("denied"),
        _ => {}
    }
    match authorization_request().await {
        Ok(Ok(_)) => permission().await,
        Ok(Err(error)) => match permission().await? {
            "denied" => Ok("denied"),
            "granted" => Ok("granted"),
            _ => Err(error),
        },
        Err(_) => Err("notification permission could not be requested".into()),
    }
}

fn authorization_request() -> oneshot::Receiver<Result<bool, String>> {
    let (send, rx) = once::<Result<bool, String>>();
    let Some(center) = center() else {
        send(Err(UNBUNDLED.into()));
        return rx;
    };
    let callback = RcBlock::new(move |granted: Bool, error: *mut NSError| {
        send(match unsafe { error_text(error) } {
            Some(message) => Err(message),
            None => Ok(granted.as_bool()),
        });
    });
    center.requestAuthorizationWithOptions_completionHandler(
        UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
        &callback,
    );
    rx
}

/// Delivers one notification now. An app without authorization gets the
/// system's error back, so a reminder is not recorded as shown.
pub(crate) async fn show(title: &str, body: &str) -> Result<(), String> {
    match delivery_request(title, body).await {
        Ok(None) => Ok(()),
        Ok(Some(message)) => Err(message),
        Err(_) => Err("notification could not be shown".into()),
    }
}

fn delivery_request(title: &str, body: &str) -> oneshot::Receiver<Option<String>> {
    let (send, rx) = once::<Option<String>>();
    let Some(center) = center() else {
        send(Some(UNBUNDLED.into()));
        return rx;
    };
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(title));
    content.setBody(&NSString::from_str(body));
    content.setSound(Some(&UNNotificationSound::defaultSound()));
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
        &NSString::from_str(&uuid::Uuid::new_v4().to_string()),
        &content,
        None,
    );
    let callback = RcBlock::new(move |error: *mut NSError| send(unsafe { error_text(error) }));
    center.addNotificationRequest_withCompletionHandler(&request, Some(&callback));
    rx
}
