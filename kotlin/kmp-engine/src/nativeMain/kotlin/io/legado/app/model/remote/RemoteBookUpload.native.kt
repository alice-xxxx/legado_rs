// 源平台来源：data/src/nativeMain/kotlin/io/legado/app/model/remote/RemoteBookUpload.native.kt。此 actual 已复制进提取模块，构建不读取源平台目录。
package io.legado.app.model.remote

import io.legado.app.lib.webdav.WebDav

/** iOS/鸿蒙本地书均为普通文件路径, 直接按路径上传。 */
internal actual suspend fun WebDav.uploadLocalBook(bookUrl: String) {
    upload(bookUrl.removePrefix("file://"))
}
