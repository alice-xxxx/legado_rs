// 源平台来源：data/src/iosMain/kotlin/io/legado/app/data/Migration84To85.ios.kt。此 actual 已复制进提取模块，构建不读取源平台目录。
package io.legado.app.data

import androidx.room3.migration.AutoMigrationSpec
import androidx.sqlite.SQLiteConnection

/**
 * iOS 端 [Migration84To85]: 官方 Room3 的 `AutoMigrationSpec.onPostMigrate` 是 suspend。
 * 清洗逻辑见 [Migration84To85Data]。
 *
 * 2026-08-04: iOS 从未发布, 无存量库, 此迁移永不触发; 保留仅为满足 expect/actual 编译(commonMain @Database 引用 spec)。
 */
actual class Migration84To85 : AutoMigrationSpec {
    override suspend fun onPostMigrate(connection: SQLiteConnection) {
        Migration84To85Data.clean(connection)
    }
}
