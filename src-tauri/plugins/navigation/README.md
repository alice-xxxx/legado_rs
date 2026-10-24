# Navigation

Independent Tauri 2 plugin for native navigation input. The consumer owns page
history, menus, and the decision to navigate or exit; the plugin has no Vue or
source-engine dependency.

Register `tauri_plugin_navigation::init()` and grant `navigation:default` to the
main window. Install the local `tauri-plugin-navigation-api` package for the
JavaScript API.

| Platform | Native integration |
| --- | --- |
| iOS | WKWebView history gestures; local edge-swipe progress and cancellation |
| Android | System button/gesture Back through Tauri's existing core listener |
| Windows | WebView2 swipe navigation settings |
| macOS | WKWebView history gestures |
| Linux | WebKitGTK history gestures |

- `setNavigationGestures(enabled, local)` enables history gestures or the iOS
  local edge recognizer. It returns `false` on Android/browser; Android system
  Back is handled separately and is not disabled by this switch.
- `supportsHistoryGestures()` identifies platforms with that native switch.
- `onBackGesture(handler)` receives iOS local gesture `start`/`move`/`end` events.
  It does not synthesize progress for platforms that do not expose it.
- `onBackRequested(handler)` receives Android system Back and Alt+Left/Cmd+[
  keyboard Back. It returns an unsubscribe function. Registering Android Back
  takes over the default behavior; the consumer must handle navigation/exit.

Native history navigation still uses the WebView's real history. Keyboard events
respect `preventDefault`; Escape remains a consumer-level dismiss action.
