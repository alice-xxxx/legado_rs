# Screen controls

Independent Tauri 2 plugin for application screen controls. Register
`tauri_plugin_screen::init()`, grant `screen:default` to the main window, and install
the local `tauri-plugin-screen-api` package. Reader/video policy stays in Vue.

```ts
import { getInfo, setSystemUiHidden, setOrientation, setKeepAwake,
  setBrightness, reset } from "tauri-plugin-screen-api";

const info = await getInfo();
await setSystemUiHidden(true);
await setOrientation("landscape");
await setKeepAwake(true);
await setBrightness(0.6);
await reset(); // restore the values saved before this plugin changed them
```

| Control | Android | iOS | Windows / macOS / Linux |
| --- | --- | --- | --- |
| Fullscreen | Immersive system bars | Status bar and content insets | Tauri window fullscreen |
| System UI visibility/style | Status/navigation bars | Status bar, with application-based appearance | Unsupported |
| Orientation | Activity orientation policy | Tauri controller policy; scene geometry on iOS 16+ | Unsupported |
| Keep display awake | Window flag | Application idle timer | Power request / IOKit assertion / session D-Bus inhibitor |
| Brightness | Window override | Foreground screen override | Unsupported |
| Window size, scale, safe area | Native metrics | Native metrics | Tauri metrics; zero content safe-area insets |

- Setters return whether the host supports and accepted that control. `getInfo()`
  reports current state and capabilities. Native errors reject the request.
- `apply(options)` updates multiple controls and returns `applied`/`unsupported`
  field lists. Fields apply independently. Fullscreen is processed before an
  explicit system-UI visibility override.
- `setOrientation` accepts `portrait`, `landscape`, their `-primary`/`-secondary`
  variants, `auto`, and `restore`. The host configuration and device policy still
  determine permitted directions. Rotation completes asynchronously;
  `onError(handler)` reports deferred iOS geometry errors and returns an
  unsubscribe function.
- `setSystemUiStyle("light" | "dark" | "auto")` controls icon/text appearance.
  iOS uses the existing app setting
  `UIViewControllerBasedStatusBarAppearance = false`; capabilities report this
  requirement. Orientation control reuses the existing Tauri controller setter.
- `setBrightness` accepts `0..1`, or `null` to restore the saved value. Android
  changes only the application window. iOS restores system brightness while the
  app is inactive and reapplies the override when active.
- `reset()` restores only values changed through this plugin. Desktop display
  inhibitors are also released when their owning window closes. Forced system
  sleep and user locking remain controlled by the OS.
- Dimensions and safe-area insets use logical pixels/points. `scale` converts
  them to physical pixels. Insets describe the current native content view.

Desktop monitor brightness/rotation are outside the application window's scope;
the API reports them as unsupported. Linux keep-awake requires a session
ScreenSaver or GNOME SessionManager service and reports an error if neither is
available. Other window operations use Tauri's existing window API directly.
