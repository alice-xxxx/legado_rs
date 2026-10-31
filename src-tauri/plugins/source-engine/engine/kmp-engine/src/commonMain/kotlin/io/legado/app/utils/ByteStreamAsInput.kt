package io.legado.app.utils

import io.legado.app.help.http.KmpResponseBody

fun Any.byteStreamAsInput(): InputStream = (this as KmpResponseBody).byteStream()
