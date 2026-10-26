import AVFAudio
import MediaPlayer
import Tauri
import UIKit
import WebKit

private struct MediaState: Decodable {
    let id: Double
    let title: String
    let artist: String
    let album: String
    let artwork: String
    let actions: [String]
    let paused: Bool
    let position: Double
    let duration: Double
    let rate: Double
}
private struct UpdateArgs: Decodable { let state: MediaState }
private struct ClearArgs: Decodable { let id: Double }

final class SystemMediaPlugin: Plugin {
    private var state: MediaState?
    private var targets: [(MPRemoteCommand, Any)] = []
    private var observers: [NSObjectProtocol] = []
    private var cover: MPMediaItemArtwork?
    private var coverTask: URLSessionDataTask?
    private var resumeAfterInterruption = false

    override func load(webview: WKWebView) {
        let notifications = NotificationCenter.default
        observers.append(notifications.addObserver(forName: AVAudioSession.interruptionNotification, object: nil, queue: .main) { [weak self] note in
            guard let self, let raw = note.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt,
                  let type = AVAudioSession.InterruptionType(rawValue: raw) else { return }
            if type == .began {
                self.resumeAfterInterruption = self.state?.paused == false
                self.emit("pause")
            } else {
                let options = AVAudioSession.InterruptionOptions(rawValue: note.userInfo?[AVAudioSessionInterruptionOptionKey] as? UInt ?? 0)
                let resume = self.resumeAfterInterruption && options.contains(.shouldResume)
                self.resumeAfterInterruption = false
                if resume { self.emit("play") }
            }
        })
        observers.append(notifications.addObserver(forName: AVAudioSession.routeChangeNotification, object: nil, queue: .main) { [weak self] note in
            if note.userInfo?[AVAudioSessionRouteChangeReasonKey] as? UInt == AVAudioSession.RouteChangeReason.oldDeviceUnavailable.rawValue {
                self?.command("pause")
            }
        })
        observers.append(notifications.addObserver(forName: AVAudioSession.mediaServicesWereResetNotification, object: nil, queue: .main) { [weak self] _ in
            self?.command("pause")
        })
    }

    private func emit(_ action: String, value: Double? = nil) {
        guard let state else { return }
        var data: JSObject = ["id": state.id, "action": action]
        if let value { data["value"] = value }
        trigger("action", data: data)
    }
    private func command(_ action: String, value: Double? = nil) {
        resumeAfterInterruption = false
        emit(action, value: value)
    }
    private func bindCommands(_ actions: [String]) {
        for (command, target) in targets { command.removeTarget(target) }
        targets.removeAll()
        let center = MPRemoteCommandCenter.shared()
        let commands: [(String, MPRemoteCommand)] = [
            ("play", center.playCommand), ("pause", center.pauseCommand), ("stop", center.stopCommand),
            ("nexttrack", center.nextTrackCommand), ("previoustrack", center.previousTrackCommand),
            ("seekto", center.changePlaybackPositionCommand), ("seekforward", center.skipForwardCommand), ("seekbackward", center.skipBackwardCommand)
        ]
        for (action, command) in commands {
            command.isEnabled = actions.contains(action)
            guard command.isEnabled else { continue }
            let target = command.addTarget { [weak self] event in
                let value = (event as? MPChangePlaybackPositionCommandEvent)?.positionTime ?? (event as? MPSkipIntervalCommandEvent)?.interval
                DispatchQueue.main.async { self?.command(action, value: value) }
                return .success
            }
            targets.append((command, target))
        }
        center.togglePlayPauseCommand.isEnabled = actions.contains("play") && actions.contains("pause")
        if center.togglePlayPauseCommand.isEnabled {
            let target = center.togglePlayPauseCommand.addTarget { [weak self] _ in
                DispatchQueue.main.async { guard let self else { return }; self.command(self.state?.paused == false ? "pause" : "play") }
                return .success
            }
            targets.append((center.togglePlayPauseCommand, target))
        }
    }

    @objc func update(_ invoke: Invoke) throws {
        let next = try invoke.parseArgs(UpdateArgs.self).state
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            if let state = self.state, next.id < state.id { invoke.resolve(); return }
            do {
                let changedSession = self.state?.id != next.id
                let changedCover = changedSession || self.state?.artwork != next.artwork
                if changedSession {
                    // WebKit 管理播放会话的激活和停用；状态同步只配置类型。
                    try AVAudioSession.sharedInstance().setCategory(.playback, mode: .default)
                    self.resumeAfterInterruption = false
                }
                if changedSession || self.state?.actions != next.actions { self.bindCommands(next.actions) }
                self.state = next
                if changedCover {
                    self.coverTask?.cancel(); self.cover = nil
                    if let url = URL(string: next.artwork), ["http", "https"].contains(url.scheme ?? "") {
                        self.coverTask = URLSession.shared.dataTask(with: url) { [weak self] data, _, _ in
                            guard let data, data.count <= 4 * 1024 * 1024, let image = UIImage(data: data) else { return }
                            DispatchQueue.main.async {
                                guard let self, self.state?.id == next.id, self.state?.artwork == next.artwork else { return }
                                self.cover = MPMediaItemArtwork(boundsSize: image.size) { _ in image }
                                self.publish()
                            }
                        }
                        self.coverTask?.resume()
                    }
                }
                self.publish()
                invoke.resolve()
            } catch { invoke.reject(error.localizedDescription) }
        }
    }
    private func publish() {
        guard let state else { return }
        var info: [String: Any] = [MPMediaItemPropertyTitle: state.title, MPMediaItemPropertyArtist: state.artist,
            MPMediaItemPropertyAlbumTitle: state.album, MPNowPlayingInfoPropertyElapsedPlaybackTime: state.position,
            MPNowPlayingInfoPropertyPlaybackRate: state.paused ? 0 : state.rate, MPNowPlayingInfoPropertyDefaultPlaybackRate: state.rate]
        if state.duration > 0 { info[MPMediaItemPropertyPlaybackDuration] = state.duration }
        if let cover { info[MPMediaItemPropertyArtwork] = cover }
        MPNowPlayingInfoCenter.default().nowPlayingInfo = info
    }
    @objc func clear(_ invoke: Invoke) throws {
        let id = try invoke.parseArgs(ClearArgs.self).id
        DispatchQueue.main.async { [weak self] in
            if self?.state?.id == id { self?.clearSession() }
            invoke.resolve()
        }
    }
    private func clearSession() {
        state = nil; resumeAfterInterruption = false
        coverTask?.cancel(); coverTask = nil; cover = nil
        for (command, target) in targets { command.removeTarget(target); command.isEnabled = false }
        targets.removeAll()
        MPNowPlayingInfoCenter.default().nowPlayingInfo = nil
    }
    deinit {
        observers.forEach { NotificationCenter.default.removeObserver($0) }
        clearSession()
    }
}

@_cdecl("init_plugin_system_media")
func initPlugin() -> Plugin { SystemMediaPlugin() }
