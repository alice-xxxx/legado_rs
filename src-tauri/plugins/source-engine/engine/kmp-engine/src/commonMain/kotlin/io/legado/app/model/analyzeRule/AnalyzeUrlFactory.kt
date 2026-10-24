package io.legado.app.model.analyzeRule

import io.legado.app.constant.AppConst
import io.legado.app.data.entities.BaseSource
import io.legado.app.data.entities.BookChapterLike
import kotlin.coroutines.CoroutineContext
import kotlin.coroutines.EmptyCoroutineContext

object AnalyzeUrlFactories {
    fun create(
        rawUrl: String,
        baseUrl: String = "",
        source: BaseSource? = null,
        ruleData: RuleDataInterface? = null,
        chapter: BookChapterLike? = null,
        readTimeout: Long? = null,
        callTimeout: Long? = null,
        coroutineContext: CoroutineContext = EmptyCoroutineContext,
        headerMapF: Map<String, String>? = null,
        hasLoginHeader: Boolean = true,
        selectedOptions: Map<String, String>? = null,
        variables: Map<AppConst.JsVarName, Any>? = null,
    ): AnalyzeUrlCore = AnalyzeUrlCore(
        rawUrl, baseUrl, source, ruleData, chapter, readTimeout, callTimeout,
        coroutineContext, headerMapF, hasLoginHeader, selectedOptions, variables
    )
}
