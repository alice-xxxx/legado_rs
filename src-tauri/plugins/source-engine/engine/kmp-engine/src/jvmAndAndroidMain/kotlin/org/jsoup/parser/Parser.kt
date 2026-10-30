package org.jsoup.parser

/** 保留源脚本的 Jsoup 静态入口，实体解析仍由 Ksoup 完成。 */
object Parser {
    @JvmStatic fun unescapeEntities(value:String,inAttribute:Boolean):String =
        com.fleeksoft.ksoup.parser.Parser.unescapeEntities(value,inAttribute)
}
