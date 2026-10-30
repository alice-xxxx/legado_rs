package io.legado.sourceengine

import io.legado.app.data.entities.BaseSource
import io.legado.app.data.entities.BookSource
import io.legado.app.help.http.StrResponse
import io.legado.app.help.source.VerificationUiProvider
import io.legado.app.help.source.VerificationUiProviders
import io.legado.app.model.analyzeRule.AnalyzeRuleCore
import io.legado.app.model.analyzeRule.AnalyzeUrlCore
import io.legado.sourceengine.host.HostRuntime
import kotlin.test.*

class SourceCompatibilityTest {
    init {
        HostRuntime.install { error("Image operations are outside this fixture") }
    }
    private fun eval(js:String) = AnalyzeRuleCore().evalJS(js)?.toString()

    @Test fun scriptCompletionAndTopLevelReturn() {
        assertEquals("42", eval("var n=40; n+2"))
        assertEquals("42", eval("if(true){return 42;} 'unused'"))
        assertEquals("42", eval("if(false){return 0;} 42;"))
        assertEquals("42", eval("if(false)return 0; var n=40; n+2;"))
        assertEquals("42", eval("{let m=' 42 ';return m.trim();}"))
        assertEquals("42", eval("function f(){return 1;} 42"))
        assertEquals("42", eval("const f=()=>{return 1;}; 42"))
        assertEquals("42", eval("const o={f(){return 1;},return:0}; o.f(); 42"))
        assertEquals("42", eval("var r=/return/; var s='return'; // return\n42"))
    }

    @Test fun urlVariablesHaveLegadoDefaults() {
        val url = AnalyzeUrlCore("https://example.org/?q={{key || 'default'}}")
        assertEquals("https://example.org/?q=default", url.url)
        assertEquals("true", url.evalJS("key===null && page===null && speakText===null && speakSpeed===null && infoMap===null").toString())
        assertEquals("true", AnalyzeRuleCore().evalJS("key===null && page===null && speakText===null && speakSpeed===null").toString())
    }

    @Test fun listsReturnedAsJsonAreParsed() {
        val rule = AnalyzeRuleCore().setContent("response")
        assertEquals(2, rule.getElements("@js:JSON.stringify([{name:'One'},{name:'Two'}])").size)
        assertEquals(0, rule.getElements("@js:'[]'").size)
    }

    @Test fun jsObjectFieldsDispatchJsonPathAndExecuteFollowingScripts() {
        val rule = AnalyzeRuleCore()
        val item = rule.evalJS("({id:42,title:'Chapter',tags:['One','Two'],nested:{name:'Name'}})")!!
        rule.setContent(item)
        assertEquals("42", rule.getString("$.id"))
        assertEquals("Name", rule.getString("$.nested.name"))
        assertEquals("Chapter!", rule.getString("title@js:result+'!'"))
        assertEquals("42/Chapter", rule.getString("@js:result.id+'/'+result.title"))
        assertEquals(listOf("One", "Two"), rule.getStringList("$.tags"))
        assertEquals(listOf("One!", "Two!"), rule.getStringList("$.tags@js:Array.from(result).map(x=>x+'!')"))
    }

    @Test fun entityDecodingPreservesUrlQueryNames() {
        val rule = AnalyzeRuleCore().setContent("<a href='/read?id=1&amp;section_slot=0&amp;chapter_slot=2'>A &amp; B</a>")
        assertEquals("/read?id=1&section_slot=0&chapter_slot=2", rule.getString("a@href"))
        assertEquals("A & B", rule.getString("a@text"))
        assertEquals("&notable=1&copycat=2", io.legado.app.utils.EscapeUtils.unescapeHtml("&notable=1&copycat=2"))
    }

    @Test fun rulesCanReadTextNodesFromTheirCurrentRow() {
        val rule=AnalyzeRuleCore().setContent("<p>Title<br>123<br>45</p>")
        assertEquals("123:45",rule.getString("@js:let tags=java.getString('p@textNodes');let [eye,star]=String(tags).trim().split('\\n').filter(d=>+d);eye+':'+star;"))
        assertEquals("Tag",rule.getString("@js:let els=java.getElements('p');cache.putMemory('compat.elements',els.outerHtml());let tags=[];els.forEach(a=>tags.push('Tag'));tags.join(',');"))
        assertEquals("Title", AnalyzeRuleCore().setContent("<h3><a>Title</a></h3>").getString(".//h3/a/text()"))
    }

    @Test fun directoryScriptsCanSelectAndIndexTheSameNodesRepeatedly() {
        val rule=AnalyzeRuleCore().setContent("<ul class='nav-tabs'><li>Line</li></ul><ul class='playlist'><li><a href='/play'>Episode</a></li></ul>")
        assertEquals("Line",rule.evalJS("var n=java.getElements('class.nav-tabs@li').length;java.getElements('class.nav-tabs@li')[n-1].text();"))
        assertEquals(1,rule.getElements("<js>var x=java.getElements('class.playlist')[0];JSON.parse(JSON.stringify(x));</js>class.playlist@li@a").size)
    }

    @Test fun publicJavaMethodsWorkOnInternalImplementations() {
        assertEquals("POST", eval("var u=new Packages.java.net.URL('https://example.org/');var c=u.openConnection();c.setRequestMethod('POST');String(c.getRequestMethod());"))
        assertEquals("2", eval("var x=Packages.java.util.Collections.unmodifiableList(Packages.java.util.Arrays.asList('a','b'));String(x.size());"))
        assertEquals("A & B", eval("Packages.org.jsoup.parser.Parser.unescapeEntities('A &amp; B',true)"))
        assertEquals("1", eval("var els=new org.jsoup.select.Elements();els.add(new org.jsoup.nodes.Element('div').text('Value'));String(els.size())"))
        assertEquals("value",eval("String(new java.lang.String('value'))"))
        assertEquals("中文",eval("java.bytesToStr(String('中文').getBytes())"))
        assertEquals("中文",eval("java.bytesToStr('中文'.getBytes('GBK'),'GBK')"))
        assertEquals("undefined",eval("typeof java.unknownHelper"))
        assertEquals("true",eval("Packages.org.jsoup.Jsoup.parse('<a href=\"javascript:test\">Link</a>').selectFirst('a').attr('href').contains('javascript')"))
        assertEquals("<p>Value</p>",eval("JSON.parse(JSON.stringify(Packages.org.jsoup.Jsoup.parse('<p>Value</p>').selectFirst('p')))"))
    }

    @Test fun androidChaChaParametersDecryptOnDesktop() {
        val key=javax.crypto.spec.SecretKeySpec(ByteArray(32),"ChaCha20")
        val cipher=javax.crypto.Cipher.getInstance("ChaCha20-Poly1305")
        cipher.init(javax.crypto.Cipher.ENCRYPT_MODE,key,javax.crypto.spec.IvParameterSpec(ByteArray(12)))
        val payload=java.util.Base64.getEncoder().encodeToString(cipher.doFinal("verified".toByteArray()))
        assertEquals("verified",eval("var C=Packages.javax.crypto.Cipher;var c=C.getInstance('ChaCha20-Poly1305');var b=Packages.java.util.Base64.getDecoder();var k=new Packages.javax.crypto.spec.SecretKeySpec(b.decode('AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA='),'ChaCha20');var p=new Packages.javax.crypto.spec.GCMParameterSpec(128,b.decode('AAAAAAAAAAAAAAAA'));c.init(C.DECRYPT_MODE,k,p);String(new Packages.java.lang.String(c.doFinal(b.decode('$payload')),'UTF-8'));"))
    }

    @Test fun browserOverloadsAndDefaultRefetch() {
        val previous = VerificationUiProviders.current()
        val calls = mutableListOf<String>()
        VerificationUiProviders.register(object:VerificationUiProvider {
            override fun awaitBrowser(source:BaseSource,url:String,title:String,refetchAfterSuccess:Boolean,html:String?):StrResponse {
                calls += "$refetchAfterSuccess:$html"
                return StrResponse(url,"verified")
            }
            override fun startBrowser(source:BaseSource,url:String,title:String,saveResult:Boolean?,refetchAfterSuccess:Boolean?,asBottomSheet:Boolean,html:String?) { calls += "open:$html:$asBottomSheet" }
            override fun getVerificationCode(url:String,source:BaseSource) = "1234"
            override fun showLoginDialog(source:BaseSource) = Unit
            override fun showSourceVariableDialog(source:BaseSource) = Unit
        })
        try {
            val source = BookSource(bookSourceUrl="https://example.org/")
            source.evalJS("java.startBrowserAwait(baseUrl,'verify');java.startBrowserAwait(baseUrl,'verify',false,'<p>html</p>');java.startBrowser(baseUrl,'show','<b>html</b>');java.startBrowser(baseUrl,'show',true);java.reLoginView(false);java.upLoginData(null);")
            assertEquals(listOf("true:null","false:<p>html</p>","open:<b>html</b>:false","open:null:true"), calls)
        } finally { VerificationUiProviders.register(previous) }
    }
}
