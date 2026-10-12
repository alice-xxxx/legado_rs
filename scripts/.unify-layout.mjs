import fs from 'node:fs';
import path from 'node:path';
import postcss from 'postcss';
const group=process.argv[2];
let patch='*** Begin Patch\n',files=0,fontChanges=0;
function hunks(before,after){
  const a=before.split(/\r?\n/),b=after.split(/\r?\n/),rows=Array.from({length:a.length+1},()=>new Uint16Array(b.length+1));
  for(let i=a.length-1;i>=0;i--)for(let j=b.length-1;j>=0;j--)rows[i][j]=a[i]===b[j]?rows[i+1][j+1]+1:Math.max(rows[i+1][j],rows[i][j+1]);
  const diff=[];let i=0,j=0;
  while(i<a.length||j<b.length){if(i<a.length&&j<b.length&&a[i]===b[j]){diff.push([' ',a[i]]);i++;j++;}else if(j<b.length&&(i===a.length||rows[i][j+1]>=rows[i+1][j]))diff.push(['+',b[j++]]);else diff.push(['-',a[i++]]);}
  const ranges=[];
  for(let k=0;k<diff.length;k++)if(diff[k][0]!==' '){const start=Math.max(0,k-3),end=Math.min(diff.length,k+4),last=ranges.at(-1);if(last&&start<=last[1])last[1]=end;else ranges.push([start,end]);}
  return ranges.map(([start,end])=>'@@\n'+diff.slice(start,end).map(([kind,line])=>kind+line).join('\n')+'\n').join('');
}
function role(d,size){
  const s=d.parent.selector??'';
  if(size>24||/chapter-(?:title|text|markup)|novel-reader/.test(s))return null;
  if(d.prop==='font'&&/monospace/.test(d.value))return 'code';
  if(/\bsmall\b|hint|subtitle|description|(?:^|[-_])meta(?:[-_ ]|$)|status-line|save-state|count|updated|timestamp|duration/.test(s))return 'caption';
  if(/appearance-workspace-group h2|workspace-group h2|section.* h[23]|sidebar-title|filter.*heading/.test(s))return 'body';
  const header=/topbar|top-title|page-title|__copy h2|settings-detail-title|tasks-title|editor-toolbar h2/.test(s);
  const title=/ h[123]\b|(?:title|heading).*strong|identity strong/.test(s);
  if((header&&/\bstrong\b| h[123]\b|title/.test(s))||size===17||size===18)return 'heading';
  if(title&&(/modal|dialog|confirm|source-import|source-login|discard|dictionary/.test(s)||size>=20))return 'title';
  if(size>=20)return 'title';
  if(title&&size>=14)return 'heading';
  if(/\bbutton\b|\binput\b|\bselect\b|\blabel\b|button|__button|action|tab|query-copy strong|list-copy strong|copy strong|task-name|result-title/.test(s))return 'body';
  return size<=13?'caption':'body';
}
const pageBodies=new Set(['.appearance-workspace','.backup-content','.home-config-editor__body','.library-import-body']);
function convert(style,file){
  const ast=postcss.parse(style);
  ast.walkRules(rule=>{
    if(file.endsWith('/MePage.vue')&&/me-prototype/.test(rule.selector)){rule.remove();return;}
    if(file.endsWith('/OtherSettingsPage.vue')&&/^\.settings-link-|^\.settings-group-|^\.other-home-content/.test(rule.selector))rule.remove();
  });
  ast.walkDecls(d=>{
    const s=d.parent.selector??'';
    if(d.prop==='font-size'||d.prop==='font'){
      const match=d.value.match(/(\d+(?:\.\d+)?)px/);
      if(match){const r=role(d,Number(match[1]));if(r){d.value=d.value.replace(match[0],'var(--ui-font-'+r+')');fontChanges++;}}
    }
    if(d.prop==='padding-inline'&&/topbar|subpage-header|import-subpage-header|settings-detail-topbar|editor-toolbar|editor-header/.test(s))d.value='var(--ui-header-gutter)';
    if(d.prop==='min-height'&&/topbar|subpage-header|import-subpage-header|settings-detail-topbar/.test(s)&&/^\d+px$/.test(d.value))d.value='var(--ui-header-height)';
    if(d.prop==='width'&&pageBodies.has(s))d.value='100%';
    if(d.prop==='padding'&&pageBodies.has(s))d.value='var(--ui-space-6) var(--ui-page-gutter) var(--ui-space-8)';
    if(d.prop==='padding'&&s==='.me-page-main')d.value=d.value.includes('67px')?'0 0 calc(var(--ui-nav-height) + var(--safe-bottom))':'0 0 var(--safe-bottom)';
  });
  for(const rule of ast.nodes.filter(n=>n.type==='rule'&&pageBodies.has(n.selector)))if(!rule.nodes.some(d=>d.prop==='max-width'))rule.append({prop:'max-width',value:'var(--app-content-width)'});
  ast.walkAtRules(rule=>{if(rule.nodes?.length===0)rule.remove();});
  return ast.toString();
}
function walk(dir){for(const e of fs.readdirSync(dir,{withFileTypes:true})){
  const p=path.join(dir,e.name),file=p.replaceAll('\\','/');
  if(e.isDirectory()){walk(p);continue;}if(!/\.(css|vue)$/.test(p))continue;
  const belongs=group==='shared'?/src\/(?:ui\/|app\/|style\.css)/.test(file):file.includes('/features/'+group+'/');
  if(!belongs)continue;
  const before=fs.readFileSync(p,'utf8');
  let after=p.endsWith('.css')?convert(before,file):before.replace(/(<style[^>]*>)([\s\S]*?)(<\/style>)/g,(_,a,b,c)=>a+convert(b,file)+c);
  if(file.endsWith('/MePage.vue')){
    const names={'me-prototype-list':'ui-page-content settings-menu','me-prototype-section':'settings-menu-section','me-prototype-section-label':'settings-menu-label','me-prototype-list-row':'settings-menu-row','me-prototype-list-icon':'settings-menu-icon','me-prototype-list-copy':'settings-menu-copy'};
    after=after.replace(/class="([^"]+)"/g,(_,classes)=>'class="'+classes.split(' ').map(c=>names[c]??c).join(' ')+'"');
  }
  if(file.endsWith('/OtherSettingsPage.vue')){
    const names={'other-home-content':'ui-page-content settings-menu','settings-link-group':'settings-menu-section','settings-group-label':'settings-menu-label','settings-link-row':'settings-menu-row','settings-link-icon':'settings-menu-icon'};
    after=after.replace(/class="([^"]+)"/g,(_,classes)=>'class="'+classes.split(' ').map(c=>names[c]??c).join(' ')+'"').replaceAll('<span><strong>','<span class="settings-menu-copy"><strong>');
  }
  if(before!==after){files++;patch+='*** Update File: '+file+'\n'+hunks(before,after);}
}}
walk('src');patch+='*** End Patch';process.stdout.write(JSON.stringify({patch,files,fontChanges}));
