package io.legado.app.model.analyzeRule

import io.legado.app.data.entities.BaseSource

object AnalyzeRuleFactories {
    fun create(
        ruleData: RuleDataInterface? = null,
        source: BaseSource? = null,
        preUpdateJs: Boolean = false,
    ): AnalyzeRuleCore = AnalyzeRuleCore(ruleData, source, preUpdateJs)
}
