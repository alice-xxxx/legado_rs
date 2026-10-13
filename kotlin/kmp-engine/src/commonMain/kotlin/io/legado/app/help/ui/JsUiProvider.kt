package io.legado.app.help.ui

import kotlin.concurrent.Volatile

/** JS 发出的 UI 更新由宿主接收，规则层不持有宿主界面。 */
interface JsUiProvider {
    fun refresh(target: String)
    fun updateLoginData(data: Map<String, String>)
    fun copy(text: String)
}

object JsUiProviders {
    @Volatile var impl: JsUiProvider? = null
}
