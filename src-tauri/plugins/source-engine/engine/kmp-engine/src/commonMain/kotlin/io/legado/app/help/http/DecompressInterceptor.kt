package io.legado.app.help.http

/** Shared transparent gzip/deflate handling for host responses. */
object DecompressInterceptor : KmpInterceptor {
    override fun intercept(chain: KmpInterceptorChain): KmpResponse {
        val request = chain.request()
        val requestBuilder = request.newBuilder()

        var transparentDecompress = false
        if (request.header("Accept-Encoding") == null && request.header("Range") == null) {
            transparentDecompress = true
            requestBuilder.header("Accept-Encoding", "gzip, deflate")
        }

        val response = chain.proceed(requestBuilder.build())
        val body = response.body

        if (!transparentDecompress || !response.promisesBody() || body == KmpResponseBody.EMPTY) {
            return response
        }

        val encoding = response.header("Content-Encoding")?.lowercase()
        val source = contentDecodingSource(body.source(), encoding) ?: return response

        return response.newBuilder()
            .removeHeader("Content-Encoding")
            .removeHeader("Content-Length")
            .body(source.asKmpResponseBody(body.contentType(), -1))
            .build()
    }
}
