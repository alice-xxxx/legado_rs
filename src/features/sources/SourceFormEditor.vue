<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from "vue";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import { bookSourceTypeOptions } from "./sourceTypeOptions";
import { setSourceJsonValue, sourceJsonValue, sourceFormFieldJsonValue } from "./sourceJsonEdits";
type Field={key:string;label:string;hint:string;wide?:boolean};
const props=defineProps<{jsonText:string;enabled:boolean;disabled:boolean}>();
const emit=defineEmits<{"update:jsonText":[value:string];pending:[value:boolean]}>();
const groups:ReadonlyArray<{title:string;fields:readonly Field[]}>= [
 {title:"基本信息",fields:[
   {key:"bookSourceName",label:"名称",hint:"bookSourceName"},
   {key:"bookSourceType",label:"内容类型",hint:"Legado 内容类型代码"},
   {key:"bookSourceGroup",label:"分组",hint:"bookSourceGroup · 可选"},
   {key:"bookSourceUrl",label:"基础地址",hint:"bookSourceUrl · 必填"},
   {key:"bookSourceComment",label:"备注",hint:"bookSourceComment · 可选",wide:true}
 ]},
 {title:"网络请求",fields:[
   {key:"legadoRsUserAgentOverride",label:"User-Agent",hint:"应用专用的 UA 覆盖 · 留空跟随来源原始 header"},
   {key:"header",label:"请求头 JSON / 规则",hint:"header · 原值为对象或数组时保留其 JSON 类型",wide:true},
   {key:"loginUrl",label:"登录地址",hint:"loginUrl · 可选"}
 ]},
 {title:"解析能力",fields:[
   {key:"searchUrl",label:"搜索 URL",hint:"searchUrl",wide:true},
   {key:"ruleSearch",label:"搜索规则",hint:"ruleSearch",wide:true},
   {key:"ruleBookInfo",label:"详情规则",hint:"ruleBookInfo",wide:true},
   {key:"ruleToc",label:"目录规则",hint:"ruleToc",wide:true},
   {key:"ruleContent",label:"正文规则",hint:"ruleContent",wide:true},
   {key:"ruleExplore",label:"发现规则",hint:"ruleExplore · 可选",wide:true}
 ]}
];
const fields=groups.flatMap(group=>group.fields);
const root=computed<Record<string,unknown>|null>(()=> {
 try {const v:unknown=JSON.parse(props.jsonText);return v && typeof v==="object"&&!Array.isArray(v)?v as Record<string,unknown>:null;}
 catch{return null;}
});
const enableKey=computed(()=>root.value && Object.prototype.hasOwnProperty.call(root.value,"bookSourceEnabled")?"bookSourceEnabled":
  root.value && Object.prototype.hasOwnProperty.call(root.value,"enabled")?"enabled":"bookSourceEnabled");
function display(key:string):string {
 const value=key==="bookSourceEnabled"?root.value?.[enableKey.value]:root.value?.[key];
 if(key==="bookSourceEnabled")return String(value===undefined?props.enabled:value!==false);
 if(value===null||value===undefined)return "";
 // Keep the original object/array/number/bool JSON token as the form baseline.
 // String fields are text inputs and deliberately show their decoded text.
 return typeof value==="string"?value:sourceJsonValue(props.jsonText,[key]);
}
const draft=ref<Record<string,string>>({});
const baselineDraft=ref<Record<string,string>>({});
const error=ref("");
const upstreamChanged=ref(false);
let baselineJsonText=props.jsonText;
let expectedAppliedJson:string|null=null;
function init():void {
 const next:Record<string,string>={};
 for(const field of fields)next[field.key]=display(field.key);
 next.bookSourceEnabled=display("bookSourceEnabled");
 draft.value=next;
 baselineDraft.value={...next};
 baselineJsonText=props.jsonText;
 upstreamChanged.value=false;
 error.value="";
}
init();
const pending=computed(()=>fields.some(field=>draft.value[field.key]!==baselineDraft.value[field.key])
  ||draft.value.bookSourceEnabled!==baselineDraft.value.bookSourceEnabled);
watch(()=>props.jsonText,newJson=>{
 if(newJson===baselineJsonText)return;
 if(newJson===expectedAppliedJson){
   expectedAppliedJson=null;
   init();
   return;
 }
 if(pending.value){
   upstreamChanged.value=true;
   return;
 }
 init();
});
watch(pending,value=>emit("pending",value),{immediate:true,flush:"sync"});
onUnmounted(()=>emit("pending",false));
function apply():void {
 if(props.disabled||!root.value)return;
 if(upstreamChanged.value){
   error.value="原始 JSON 已在表单编辑期间更新，请先重新读取字段并确认修改。";
   return;
 }
 let next=props.jsonText;
 try {
   for(const field of fields){
     const key=field.key,text=draft.value[key]??"";
     if(text===display(key))continue;
     const raw=sourceFormFieldJsonValue(key,field.label,root.value[key],text);
     next=setSourceJsonValue(next,[key],raw);
   }
   if(draft.value.bookSourceEnabled!==display("bookSourceEnabled")){
     const enabled=draft.value.bookSourceEnabled==="true";
     next=setSourceJsonValue(next,[enableKey.value],enabled?"true":"false");
     if(Object.prototype.hasOwnProperty.call(root.value,"enabled") && enableKey.value!=="enabled")
       next=setSourceJsonValue(next,["enabled"],enabled?"true":"false");
     if(Object.prototype.hasOwnProperty.call(root.value,"bookSourceEnabled") && enableKey.value!=="bookSourceEnabled")
       next=setSourceJsonValue(next,["bookSourceEnabled"],enabled?"true":"false");
   }
   error.value="";
   expectedAppliedJson=next;
   emit("update:jsonText",next);
 } catch (cause) { error.value=cause instanceof Error?cause.message:String(cause); }
}
</script>
<template>
 <div class="source-visual-form">
   <header class="source-visual-heading">
     <div><strong>{{draft.bookSourceName||'内容引擎'}}</strong></div>
     <label>启用 <input type="checkbox" :checked="draft.bookSourceEnabled==='true'" :disabled="disabled"
       @change="draft.bookSourceEnabled=($event.target as HTMLInputElement).checked?'true':'false'"/></label>
   </header>
   <section v-for="group in groups" :key="group.title" class="source-visual-section">
     <header><strong>{{group.title}}</strong></header>
     <div class="source-visual-fields">
       <label v-for="field in group.fields" :key="field.key" :class="{'wide':field.wide}">
         <span>{{field.label}}</span>
         <select v-if="field.key==='bookSourceType'" :value="draft.bookSourceType??''" :disabled="disabled"
           @change="draft.bookSourceType=($event.target as HTMLSelectElement).value">
           <option v-for="option in bookSourceTypeOptions(draft.bookSourceType??'')" :key="option.value" :value="option.value">{{ option.label }}</option>
         </select>
         <textarea v-else-if="field.wide" :value="draft[field.key]??''" spellcheck="false" :disabled="disabled"
           @input="draft[field.key]=($event.target as HTMLTextAreaElement).value"></textarea>
         <input v-else :value="draft[field.key]??''" :disabled="disabled" spellcheck="false" inputmode="text"
           @input="draft[field.key]=($event.target as HTMLInputElement).value"/>
         <small>{{field.hint}}</small>
       </label>
     </div>
   </section>
   <p v-if="upstreamChanged" class="source-visual-error">书源 JSON 在编辑过程中发生了外部更新。当前表单草稿仍保留，但不能直接覆盖新版本；请点击「重新读取字段」再继续编辑。</p>
   <p v-if="error" class="source-visual-error">{{error}}</p>
   <footer class="source-visual-actions">
     <button type="button" class="button secondary small" :disabled="disabled||(!pending&&!upstreamChanged)" @click="init">{{ upstreamChanged ? '重新读取字段' : '撤销表单修改' }}</button>
     <button type="button" class="button primary small"
       :disabled="disabled||!pending||upstreamChanged" @click="apply"><PrototypeIcon name="right"/> 应用修改</button>
   </footer>
 </div>
</template>
<style scoped>
.source-visual-form{display:grid;gap:0;min-width:0;color:#343640}
.source-visual-heading{display:flex;align-items:center;justify-content:space-between;gap:12px;padding:2px 0 22px}
.source-visual-heading>div{display:grid;min-width:0;gap:4px}
.source-visual-heading strong{overflow:hidden;white-space:nowrap;text-overflow:ellipsis;font-size:17px;color:#272934}
.source-visual-heading>label{display:flex;align-items:center;gap:8px;white-space:nowrap;color:#6b707b;font-size:9px}
.source-visual-heading input{width:18px;height:18px;accent-color:var(--app-accent)}
.source-visual-section{padding:17px 0 18px;margin:0;border:0;border-top:1px solid var(--app-line);border-radius:0;background:transparent}
.source-visual-section>header{display:grid;gap:4px;margin:0 0 14px;padding:0;border:0}
.source-visual-section>header strong{font-size:11px;color:#343640}
.source-visual-fields{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:10px 12px}
.source-visual-fields>label{display:grid;align-content:start;gap:6px;min-width:0}
.source-visual-fields>label.wide{grid-column:1/-1}
.source-visual-fields>label>span{color:#777c85;font-size:9px;font-weight:600}
.source-visual-fields>label>input,.source-visual-fields>label>textarea,.source-visual-fields>label>select{box-sizing:border-box;width:100%;min-width:0;min-height:36px;padding:8px 9px;border:1px solid #e1e4eb;border-radius:9px;background:#fff;color:#353743;font-family:inherit;font-size:10px;line-height:1.5}
.source-visual-fields>label>select{cursor:pointer}
.source-visual-fields>label>textarea{min-height:82px;resize:vertical;font:12px/1.55 ui-monospace,SFMono-Regular,Consolas,monospace}
.source-visual-fields>label>input:focus,.source-visual-fields>label>textarea:focus,.source-visual-fields>label>select:focus{outline:2px solid #6757d644;outline-offset:1px;border-color:var(--app-accent)}
.source-visual-fields>label>small{font-size:8px;line-height:1.5;color:#a1a5ae}
.source-visual-error{margin:10px 0;padding:10px 12px;border-radius:9px;background:#fff0f2;color:#b0525c;font-size:12px}
.source-visual-actions{display:flex;justify-content:flex-end;align-items:center;gap:8px;flex-wrap:wrap;padding:17px 0;border-top:1px solid var(--app-line)}
.source-visual-actions .button{min-height:36px;border-radius:9px;font-size:12px}
@media(max-width:700px){.source-visual-heading{padding:5px 0 17px}.source-visual-heading strong{font-size:17px}.source-visual-fields{grid-template-columns:1fr}.source-visual-actions>small{flex-basis:100%}}
</style>
