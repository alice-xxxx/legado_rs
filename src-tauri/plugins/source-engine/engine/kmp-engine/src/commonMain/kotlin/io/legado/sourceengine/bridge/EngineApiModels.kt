package io.legado.sourceengine.bridge

import io.legado.app.data.entities.*
import io.legado.app.data.entities.rule.ExploreKind
import io.legado.app.utils.KS_JSON
import kotlinx.serialization.*
import kotlinx.serialization.descriptors.*
import kotlinx.serialization.encoding.*
import kotlinx.serialization.json.*

/** Source documents are decoded once at the native boundary into the rule model. */
object SourceDefinitionSerializer : KSerializer<BookSource> {
    override val descriptor = BookSource.serializer().descriptor
    override fun deserialize(decoder: Decoder): BookSource {
        val definition = (decoder as JsonDecoder).decodeJsonElement().jsonObject
        val legacy = "bookSourceUrl" !in definition && "sourceUrl" in definition &&
            listOf("ruleArticles", "sortUrl", "ruleTitle", "ruleLink").any { it in definition }
        val source = if (legacy) KS_JSON.decodeFromJsonElement(OldRssSource.serializer(), definition).toBookSource()
            else KS_JSON.decodeFromJsonElement(BookSource.serializer(), definition)
        val override = definition["legadoRsUserAgentOverride"] as? JsonPrimitive
        source.sourceUserAgentOverride = override?.takeIf { it.isString }?.content?.trim()
            ?.takeIf { it.isNotEmpty() && it.encodeToByteArray().size <= 512 && it.none { c -> c.code in 0..31 || c.code in 127..159 } }
        return source
    }
    override fun serialize(encoder: Encoder, value: BookSource) =
        BookSource.serializer().serialize(encoder, value)
}

@Serializable data class SourceRequest(
    @Serializable(with = SourceDefinitionSerializer::class) val source: BookSource,
)
@Serializable data class SearchRequest(
    @Serializable(with = SourceDefinitionSerializer::class) val source: BookSource,
    val keyword: String,
    val page: Int = 1,
)
@Serializable data class ExploreRequest(
    @Serializable(with = SourceDefinitionSerializer::class) val source: BookSource,
    val url: String,
    val page: Int = 1,
)
@Serializable data class BookRequest(
    @Serializable(with = SourceDefinitionSerializer::class) val source: BookSource,
    val book: Book,
)
@Serializable data class ContentRequest(
    @Serializable(with = SourceDefinitionSerializer::class) val source: BookSource,
    val book: Book,
    val chapter: BookChapter,
    val nextChapterUrl: String? = null,
)
@Serializable data class MediaRequest(
    @Serializable(with = SourceDefinitionSerializer::class) val source: BookSource,
    val book: Book,
    val chapter: BookChapter,
    val url: String,
    val headers: Map<String, String> = emptyMap(),
)
@Serializable data class TtsRequest(val config: HttpTTS, val text: String, val speechRate: Int)

@OptIn(ExperimentalSerializationApi::class)
@Serializable @JsonClassDiscriminator("kind")
sealed class LoginAction {
    @Serializable @SerialName("open") data object Open : LoginAction()
    @Serializable @SerialName("clear") data object Clear : LoginAction()
    @Serializable @SerialName("submit") data class Submit(val fields: Map<String, String>, val button: Int? = null) : LoginAction()
    @Serializable @SerialName("importCookies") data class ImportCookies(val url: String, val cookies: String) : LoginAction()
}
@Serializable data class LoginRequest(
    @Serializable(with = SourceDefinitionSerializer::class) val source: BookSource,
    val action: LoginAction,
)

@Serializable data class SearchPage(val books: List<SearchBook>, val hasNextPage: Boolean)
@Serializable data class ExploreKinds(val categories: List<ExploreKind>)
@Serializable data class ChapterList(val chapters: List<BookChapter>)
@Serializable data class ChapterContent(val content: String)
@Serializable data class MediaLocation(val url: String, val headers: Map<String, String>)
@Serializable data class AudioData(val bytesBase64: String, val contentType: String)

@Serializable data class LoginField(val name: String, val label: String, val type: String, val password: Boolean, val choices: List<String> = emptyList())
@Serializable data class LoginButton(val id: Int, val label: String)
@Serializable data class LoginLayout(val kind: String, val name: String? = null, val id: Int? = null, val label: String? = null, val cols: Int = 1, val rows: Int = 1)
@Serializable data class LoginForm(val mode: String, val canLogin: Boolean = false, val fields: List<LoginField> = emptyList(), val actions: List<LoginButton> = emptyList(), val layout: List<LoginLayout> = emptyList())
@Serializable data class LoginWeb(val url: String, val cookieHeader: String)
@Serializable data class LoginState(
    val hasLoginState: Boolean = false,
    val message: String? = null,
    val error: String? = null,
    val form: LoginForm? = null,
    val web: LoginWeb? = null,
    val messages: List<String> = emptyList(),
    val inputUpdates: Map<String, String> = emptyMap(),
    val refreshTargets: List<String> = emptyList(),
    val copyText: String? = null,
)
