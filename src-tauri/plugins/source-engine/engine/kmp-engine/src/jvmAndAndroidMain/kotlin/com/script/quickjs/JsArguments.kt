package com.script.quickjs

internal fun scriptArgument(value: Any?, dangerousApi: Boolean): String =
    JsStringUtils.primitive(value) ?: if (value != null && JsSecurityPolicy.isObjectVisible(value, dangerousApi)) {
        "__wrapJavaHandle(" + JavaObjectBridge.registerObject(value) + ")"
    } else "null"
