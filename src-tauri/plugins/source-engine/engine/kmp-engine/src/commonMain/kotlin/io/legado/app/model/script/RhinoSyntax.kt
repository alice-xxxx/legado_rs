package io.legado.app.model.script

import io.legado.app.utils.KS_JSON
import kotlinx.serialization.encodeToString

/** 只调整 Rhino 的语法扩展；字符串、模板文本、注释和正则不参与改写。 */
object RhinoSyntax {
    private val namePattern=Regex("[\\p{L}_$][\\p{L}\\p{N}_$]*")
    private val regexStart=setOf("","(","[","{","=",":",",",";","!","?","&&","||","=>","return","throw","case","yield","else")
    private val operators=setOf("=>","++","--","&&","||","??","?.","==","!=","<=",">=","**")
    private data class Token(val text:String,val start:Int,val end:Int)
    private data class Edit(val start:Int,val end:Int,val text:String)
    private class Scan(val source:String) {
        var position=0
        private var previous=""
        val edits=mutableListOf<Edit>()
        fun next():Token? {
            while(position<source.length) {
                val c=source[position]
                when {
                    c=='\u200b' -> {edits+=Edit(position,position+1,"");position++}
                    c.isWhitespace()||c=='\ufeff' -> position++
                    source.startsWith("//",position) -> {while(position<source.length&&source[position]!='\n')position++}
                    source.startsWith("/*",position) -> {position=(source.indexOf("*/",position+2).takeIf{it>=0}?.plus(2)?:source.length)}
                    else -> break
                }
            }
            if(position>=source.length)return null
            val start=position
            val c=source[position++]
            val text=when {
                c=='\''||c=='"'||c=='`' -> {quoted(c);"literal"}
                c=='/'&&previous in regexStart -> {
                    var group=false
                    while(position<source.length) {
                        val r=source[position++]
                        if(r=='\\'){position=(position+1).coerceAtMost(source.length);continue}
                        if(r=='[')group=true
                        if(r==']')group=false
                        if(r=='/'&&!group)break
                    }
                    while(position<source.length&&source[position].isLetter())position++
                    "literal"
                }
                c.isLetterOrDigit()||c=='_'||c=='$' -> {
                    while(position<source.length&&(source[position].isLetterOrDigit()||source[position] in "_$\u200c\u200d"))position++
                    source.substring(start,position)
                }
                position<source.length&&source.substring(start,position+1) in operators -> source.substring(start,++position)
                else -> c.toString()
            }
            previous=text
            return Token(text,start,position)
        }
        private fun quoted(quote:Char) {
            while(position<source.length) {
                val c=source[position++]
                if(c=='\\'){position=(position+1).coerceAtMost(source.length);continue}
                if(c==quote)return
                if(quote=='`'&&c=='$'&&position<source.length&&source[position]=='{') {
                    position++
                    var depth=1
                    previous="{"
                    while(depth>0) {
                        val token=next()?:return
                        if(token.text=="{")depth++
                        if(token.text=="}")depth--
                    }
                }
            }
        }
    }
    private fun apply(source:String,edits:List<Edit>):String {
        if(edits.isEmpty())return source
        val result=StringBuilder(source)
        edits.sortedByDescending{it.start}.forEach{result.replace(it.start,it.end,it.text)}
        return result.toString()
    }
    fun normalize(source:String):String {
        val scanner=Scan(source)
        val tokens=generateSequence{scanner.next()}.toList()
        val pairs=mutableMapOf<Int,Int>()
        val stack=mutableListOf<Int>()
        tokens.forEachIndexed{i,t->
            if(t.text in listOf("(","[","{"))stack+=i
            else if(t.text in listOf(")","]","}")&&stack.isNotEmpty()) {
                val open=stack.removeAt(stack.lastIndex)
                if("([{".indexOf(tokens[open].text)==")]}".indexOf(t.text)){pairs[i]=open;pairs[open]=i}
            }
            if(t.text=="=>"&&i>0&&tokens[i-1].text in listOf("]","}")) {
                pairs[i-1]?.let{open->
                    scanner.edits+=Edit(tokens[open].start,tokens[open].start,"(")
                    scanner.edits+=Edit(tokens[i-1].end,tokens[i-1].end,")")
                }
            }
        }
        tokens.forEachIndexed{i,t->
            if(t.text!="function")return@forEachIndexed
            val param=(i+1..minOf(i+3,tokens.lastIndex)).firstOrNull{tokens[it].text=="("}?:return@forEachIndexed
            val end=pairs[param]?:return@forEachIndexed
            val params=tokens.subList(param+1,end)
            if(params.any{it.text!=","&&!it.text.matches(namePattern)})return@forEachIndexed
            if(tokens.getOrNull(end+1)?.text!="{")return@forEachIndexed
            val bodyEnd=pairs[end+1]?:return@forEachIndexed
            val names=params.filter{it.text!=","}.map{it.text}.toSet()
            var depth=1
            for(n in end+2 until bodyEnd) {
                when(tokens[n].text) {
                    "{" -> depth++
                    "}" -> depth--
                    "let" -> if(depth==1&&tokens.getOrNull(n+1)?.text in names)scanner.edits+=Edit(tokens[n].start,tokens[n].end,"var")
                }
            }
        }
        return apply(source,scanner.edits.distinct())
    }
    /** Gson 的单引号 JSON 字符串先转成标准 JSON，其余键和值保持原样。 */
    fun json(source:String):String {
        val scanner=Scan(source)
        val edits=mutableListOf<Edit>()
        while(true) {
            val token=scanner.next()?:break
            if(source[token.start]!='\'')continue
            val value=StringBuilder()
            var i=token.start+1
            while(i<token.end-1) {
                val c=source[i++]
                if(c!='\\'||i>=token.end-1){value.append(c);continue}
                val escape=source[i++]
                when(escape) {
                    'n' -> value.append('\n')
                    'r' -> value.append('\r')
                    't' -> value.append('\t')
                    'b' -> value.append('\b')
                    'f' -> value.append('\u000c')
                    'v' -> value.append('\u000b')
                    '0' -> value.append('\u0000')
                    'u','x' -> {
                        val length=if(escape=='u')4 else 2
                        val code=source.substring(i,minOf(i+length,token.end-1)).toIntOrNull(16)
                        if(code==null)value.append(escape)else{value.append(code.toChar());i+=length}
                    }
                    '\n' -> Unit
                    '\r' -> if(i<token.end-1&&source[i]=='\n')i++
                    else -> value.append(escape)
                }
            }
            edits+=Edit(token.start,token.end,KS_JSON.encodeToString(value.toString()))
        }
        return apply(source,edits)
    }
}
