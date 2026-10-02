import Foundation
import LegadoSourceEngine
import SwiftRs
import Tauri
import UIKit
import WebKit

private struct ExecuteArgs: Decodable {
    let requestJson: String
}

/// Tauri's iOS edge enters Kotlin/Native; its HTTP and storage callbacks return through Rust C ABI.
final class SourceEnginePlugin: Plugin {
    @objc public func execute(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(ExecuteArgs.self)
        let applicationSupport = FileManager.default.urls(
            for: .applicationSupportDirectory,
            in: .userDomainMask
        ).first!
        let dataDirectory = applicationSupport.appendingPathComponent("source-engine", isDirectory: true).path

        IosSourceEngine.shared.executeJson(
            requestJson: args.requestJson,
            appDataDirectory: dataDirectory
        ) { result, error in
            if let error {
                invoke.reject(error.localizedDescription)
                return
            }
            guard let result else {
                invoke.reject("Kotlin source engine returned no result")
                return
            }
            invoke.resolve(["resultJson": result])
        }
    }
}

@_cdecl("init_plugin_source_engine")
func initPlugin() -> Plugin {
    SourceEnginePlugin()
}
