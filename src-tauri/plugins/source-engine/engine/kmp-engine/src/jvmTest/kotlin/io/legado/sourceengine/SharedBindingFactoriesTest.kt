package io.legado.sourceengine

import io.legado.app.model.script.*
import io.legado.sourceengine.host.HostRuntime
import kotlin.test.*

/** Runs production Native wrappers in QuickJS; C value ownership requires iOS validation. */
class SharedBindingFactoriesTest {
    init { HostRuntime.install { error("Image operation outside fixture") } }

    private fun eval(script: String, methods: Map<String, String> = emptyMap()): String =
        JsEngines.current.eval(nativeBindingFactories(methods) + "\n" + script).toString()

    @Test fun overloadsKeepArgumentCountsAndGeneratedMethodsOverrideInsideTheirFactory() {
        assertEquals("[[5,3,[\"x\"]],[5,13,[\"x\",null]],[5,4,[1,2]],[5,14,[[1,2],\"UTF-8\"]],[5,25,[\"key\"]],[5,309,[\"url\",\"{}\"]],[5,301,[\"url\"]],[5,301,[\"url\",null]],[5,5001,[\"value\"]]]", eval("""
            var calls = [];
            function __nativeDispatch(handle, method, args) { calls.push([handle, method, args]); return 1; }
            var java = __createJavaObj(5);
            java.strToBytes('x'); java.strToBytes('x', null);
            java.bytesToStr([1,2]); java.bytesToStr([1,2], 'UTF-8');
            java.get('key'); java.get('url', {});
            java.ajax('url'); java.ajax('url', null);
            java.base64Encode('value');
            JSON.stringify(calls);
        """, mapOf("__createJavaObj" to "    obj.base64Encode = function(v) { return __nativeDispatch(handle, 5001, [v]); };",
            "unknownFactory" to "throw new Error('unknown factory must be ignored');")))
    }

    @Test fun entityPropertiesKeepAccessorsAndReadOnlyDescriptors() {
        assertEquals("[\"original\",\"updated\",false,false,true]", eval("""
            var title = 'original';
            function __nativeDispatch(handle, method, args) {
                if (method === 1802) return title;
                if (method === 2802) title = args[0];
                return null;
            }
            var chapter = __createChapterObj(1);
            var before = chapter.title;
            chapter.title = 'updated';
            var descriptor = Object.getOwnPropertyDescriptor(chapter, 'title');
            var book = __createBookObj(2);
            JSON.stringify([before, chapter.title, descriptor.enumerable, descriptor.configurable,
                Object.getOwnPropertyDescriptor(book, 'lastChapterIndex').set === undefined]);
        """))
    }

    @Test fun imageReferencesStayOpaqueAndBoundToTheirFactory() {
        assertEquals("[7,2,0,true,true]", eval("""
            var calls = 0;
            function __nativeDispatch(handle, method, args) {
                calls++;
                if (method === 3301) return new Uint8Array([7]);
                if (method === 3302) return args[0][0];
                throw new Error('unexpected image operation');
            }
            var image = __createImageOpsObj(1), other = __createImageOpsObj(1);
            var ref = image.decode('data');
            var value = image.encode(ref, 'png');
            var rejected = false, foreignRejected = false;
            try { image.encode({}, 'png'); } catch (e) { rejected = e instanceof TypeError; }
            try { other.encode(ref, 'png'); } catch (e) { foreignRejected = e instanceof TypeError; }
            JSON.stringify([value, calls, Object.keys(ref).length, rejected, foreignRejected]);
        """))
    }
}
