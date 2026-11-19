use super::Tool;
use crate::cdp;
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use anyhow::{Context, Result};
use async_trait::async_trait;
use chromiumoxide::Page;
use serde_json::{Value, json};

/// Controls the Elementor editor via CDP by evaluating JS against Elementor's internal API (`$e.run`).
///
/// **Danger:** `$e.run` is an undocumented internal API that may change between Elementor versions.
///
/// Requires an authenticated `WordPress` session — the CDP browser must already be logged in.
/// The editor must be opened first (`action: 'open'`) before any other action will work.
///
/// Pages are **not** closed after `open` — subsequent actions address the editor URL for page_id.
pub struct ElementorEditor;

async fn editor_url(page_id: u64, wp: &WpClient) -> Result<String> {
    let runtime = wp
        .get("mcp-for-page-builders/v1/runtime")
        .await
        .context("read WordPress runtime URLs for the native editor")?;
    let admin_url = runtime["admin_url"]
        .as_str()
        .context("WordPress runtime response has no admin_url")?;
    let site_url =
        reqwest::Url::parse(wp.base_url()).context("invalid configured WordPress URL")?;
    let mut admin_url =
        reqwest::Url::parse(admin_url).context("WordPress returned an invalid admin_url")?;
    anyhow::ensure!(
        admin_url.origin() == site_url.origin(),
        "WordPress admin_url points to a different origin than the configured site"
    );
    anyhow::ensure!(
        admin_url.username().is_empty() && admin_url.password().is_none(),
        "WordPress admin_url must not contain credentials"
    );
    admin_url.set_path(&format!(
        "{}/post.php",
        admin_url.path().trim_end_matches('/')
    ));
    admin_url.set_query(Some(&format!("post={page_id}&action=elementor")));
    admin_url.set_fragment(None);
    Ok(admin_url.into())
}

async fn editor_page(page_id: u64, wp: &WpClient) -> Result<Page> {
    let url = editor_url(page_id, wp).await?;
    cdp::find_matching_page(&url).await?.ok_or_else(|| {
        anyhow::anyhow!("Elementor editor for page {page_id} is not open; call open first")
    })
}

fn js_select_widget(element_id: &str) -> String {
    let element_id =
        serde_json::to_string(element_id).expect("Elementor element IDs serialize as JSON strings");
    format!(
        r"(async()=>{{const container=elementor.getContainer({element_id});if(!container)return 'not_found';await $e.run('panel/editor/open',{{model:container.model,view:container.view}});return 'selected'}})()"
    )
}

fn js_checked_setting_save(
    page_id: u64,
    element_id: &str,
    key: &str,
    value: &Value,
    expected_revision: &str,
    operation_id: &str,
    expected_user_id: u64,
) -> Result<String> {
    let args = serde_json::to_string(&json!({
        "pageId": page_id,
        "elementId": element_id,
        "key": key,
        "value": value,
        "expectedRevision": expected_revision,
        "operationId": operation_id,
        "expectedUserId": expected_user_id,
    }))?;
    Ok(r#"(async()=>{
const args=__ARGS__;
const result=(value)=>JSON.stringify({...value,post_id:value.post_id??args.pageId,operation_id:value.operation_id??args.operationId,local_state:value.local_state??'unchanged'});
const message=(error)=>String(error?.code||error?.message||error||'editor operation failed');
const api=window.wp?.apiFetch;
const boundedApi=(options)=>{const controller=new AbortController();let timer;return new Promise((resolve,reject)=>{timer=setTimeout(()=>{controller.abort();reject(new Error('WordPress editor request timed out'));},10000);Promise.resolve(api({...options,signal:controller.signal})).then(resolve,reject);}).finally(()=>clearTimeout(timer));};
const document=elementor?.documents?.getCurrent?.();
const saveComponent=$e?.components?.get?.('document/save');
const container=document?.container;
const element=elementor?.getContainer?.(args.elementId);
const targetBelongsToDocument=()=>{const find=(model)=>{if(!model)return false;if(model===element?.model&&model.get('id')===args.elementId)return true;const children=model.get('elements');return children?.models?.some(find)===true;};return find(document?.container?.model);};
if(!api||!document||!container||!element||!targetBelongsToDocument())return result({error:'Native editor APIs or document are unavailable'});
const getElementControls=elementor?.getElementControls;
if(typeof getElementControls!=='function')return result({error:'Native Elementor control schema is unavailable'});
let controls;
try{controls=getElementControls.call(elementor,element.model);}catch(error){return result({error:'Native Elementor control schema could not be read: '+message(error)});}
if(!controls||typeof controls!=='object'||Array.isArray(controls)||!Object.prototype.hasOwnProperty.call(controls,args.key))return result({error:'The setting key is not a native control for this element'});
if(Number(document.id)!==args.pageId)return result({error:'The active editor document does not match page_id'});
if(document.editor.isSaving!==false||document.editor.isChanged!==false)return result({error:'The editor save state is unavailable, a save is in flight, or the document already has unsaved changes'});
if(typeof saveComponent?.stopAutoSave!=='function'||typeof saveComponent?.startAutoSave!=='function')return result({error:'Native autosave controls are unavailable'});
let leaseToken=null,leaseCreated=false,dispatched=false,intercepted=false,originalAddRequest=null,addRequestWrapper=null,ajax=null,originalJqAjax=null,jqAjaxWrapper=null,yoastGate=null,yoastIntercepted=false,yoastFailure=null,yoastFailureKind=null,autosaveFenceInstalled=false,gate=null,nativeErrorCalled=false,nativeCommandStarted=false;
const notifyNativeError=(context,options,error)=>{let failure=message(error);if(!nativeErrorCalled){nativeErrorCalled=true;try{options?.error?.call(context,failure,'mcp_pre_dispatch',document);}catch(callbackError){failure+='; native error callback failed: '+message(callbackError);}}return failure;};
const rejectNative=(context,options,error)=>jQuery.Deferred().reject(new Error(notifyNativeError(context,options,error))).promise();
const originalStartAutoSave=saveComponent.startAutoSave;
const fencedStartAutoSave=function(target){if(target===document){saveComponent.stopAutoSave(target);return;}return Reflect.apply(originalStartAutoSave,this,arguments);};
let previousHad=Object.prototype.hasOwnProperty.call(element.settings.attributes,args.key);
let previousValue=element.settings.get(args.key);
let changed=false;
const settingsPayload=(status)=>{
 const settings=container.settings.toJSON({remove:['default']});
 (elementor.config.persistent_keys||[]).forEach((setting)=>{
  if(Object.prototype.hasOwnProperty.call(container.settings.defaults,setting)&&!Object.prototype.hasOwnProperty.call(settings,setting))settings[setting]=container.settings.defaults[setting];
 });
 settings.post_status=status;
 return settings;
};
const payload=(status)=>({status,elements:elementor.config.document.panel.has_elements?container.model.get('elements').toJSON({remove:['default','editSettings','defaultEditSettings']}):[],settings:settingsPayload(status)});
const same=(left,right)=>JSON.stringify(left)===JSON.stringify(right);
const beforeStatus=container.settings.get('post_status')||'draft';
if(!['draft','publish'].includes(beforeStatus))return result({error:'Only draft and publish documents support checked native saves'});
const yoastForm=window.document.querySelector('#yoast-form');
let nativeYoast=null;
if(yoastForm){
 const nativeAction=yoastForm.querySelector('input[name=action]');
 const nativePost=yoastForm.querySelector('input[name=post_id]');
 let formUrl;try{formUrl=new URL(yoastForm.getAttribute('action'),location.href);}catch(error){return result({error:'The native Yoast form action is invalid'});}
 const sameOrigin=formUrl.origin===location.origin;
 const nativeNonce=yoastForm.querySelector('input[name=_wpseo_elementor_nonce]');
 let ajaxUrl;try{ajaxUrl=new URL(window.ajaxurl,location.href);}catch(error){}
 if(yoastForm.method.toLowerCase()!=='post'||nativeAction?.value!=='wpseo_elementor_save'||nativePost?.value!==String(args.pageId)||!nativeNonce?.value||!sameOrigin||!ajaxUrl||ajaxUrl.href!==formUrl.href||!formUrl.pathname.endsWith('/wp-admin/admin-ajax.php')||elementor?.documents?.getCurrent?.()!==document||Number(document.id)!==args.pageId)return result({error:'The native Yoast form does not match the active editor document'});
 nativeYoast={form:yoastForm,url:formUrl.href,nonce:nativeNonce.value};
}
const nativeYoastContextValid=()=>!nativeYoast||(window.document.querySelector('#yoast-form')===nativeYoast.form&&nativeYoast.form.querySelector('input[name=action]')?.value==='wpseo_elementor_save'&&nativeYoast.form.querySelector('input[name=post_id]')?.value===String(args.pageId)&&Boolean(nativeYoast.form.querySelector('input[name=_wpseo_elementor_nonce]')?.value)&&nativeYoast.form.method.toLowerCase()==='post'&&(()=>{try{return new URL(nativeYoast.form.getAttribute('action'),location.href).href===nativeYoast.url;}catch(error){return false;}})());
let before,editedPayload;
try{
 const user=await boundedApi({path:'/wp/v2/users/me?context=edit'});
 if(!Number.isSafeInteger(user?.id)||user.id!==args.expectedUserId)return result({error:'The browser user does not match the configured WordPress user'});
 const revision=await boundedApi({path:'/mcp-for-page-builders/v1/document/'+args.pageId});
 if(revision?.revision!==args.expectedRevision)return result({error:'Stale document revision; read the document again'});
 if(elementor?.documents?.getCurrent?.()!==document||Number(document.id)!==args.pageId||elementor?.getContainer?.(args.elementId)!==element||!targetBelongsToDocument()||!nativeYoastContextValid()||document.editor.isSaving!==false||document.editor.isChanged!==false)return result({error:'The active document, target element, or native save state changed during revision verification'});
 before=payload(beforeStatus);
 saveComponent.startAutoSave=fencedStartAutoSave;
 autosaveFenceInstalled=true;
 saveComponent.stopAutoSave(document);
 changed=true;
 await $e.run('document/elements/settings',{container:element,settings:{[args.key]:args.value}});
 editedPayload=payload(beforeStatus);
 saveComponent.stopAutoSave(document);
 ajax=elementorCommon?.ajax;
 if(typeof ajax?.addRequest!=='function')throw new Error('Native Elementor save transport is unavailable');
 originalAddRequest=ajax.addRequest;
 gate=jQuery.Deferred();
 if(nativeYoast){
  if(typeof jQuery?.ajax!=='function')throw new Error('Native Yoast AJAX transport is unavailable');
  originalJqAjax=jQuery.ajax;
  yoastGate=jQuery.Deferred();
  const readFields=(data)=>{if(typeof data==='string'||data instanceof URLSearchParams){const params=data instanceof URLSearchParams?new URLSearchParams(data):new URLSearchParams(data);if(params.getAll('action').length>1||params.getAll('post_id').length>1||params.getAll('_wpseo_elementor_nonce').length>1)return null;return {action:params.get('action'),post_id:params.get('post_id'),nonce:params.get('_wpseo_elementor_nonce'),kind:data instanceof URLSearchParams?'urlsearch':'search',data:params};}if(Array.isArray(data)){const fields=new Map();for(const entry of data){if(!entry||typeof entry.name!=='string')return null;if(['action','post_id','_wpseo_elementor_nonce'].includes(entry.name)){if(fields.has(entry.name))return null;fields.set(entry.name,entry.value);}}return {action:fields.get('action'),post_id:fields.get('post_id'),nonce:fields.get('_wpseo_elementor_nonce'),kind:'array',data};}if(data&&typeof data==='object'&&!(data instanceof FormData)&&!(data instanceof URLSearchParams)){return {action:data.action,post_id:data.post_id,nonce:data._wpseo_elementor_nonce,kind:'object',data};}return null;};
  const attachToken=(fields)=>{if(fields.kind==='search'||fields.kind==='urlsearch'){if(fields.data.has('mcp_guard_token'))throw new Error('The native Yoast request already contains a guard token');fields.data.append('mcp_guard_token',leaseToken);return fields.kind==='urlsearch'?fields.data:fields.data.toString();}if(fields.kind==='array'){if(fields.data.some(entry=>entry.name==='mcp_guard_token'))throw new Error('The native Yoast request already contains a guard token');return [...fields.data,{name:'mcp_guard_token',value:leaseToken}];}if(fields.kind==='object'){if(Object.prototype.hasOwnProperty.call(fields.data,'mcp_guard_token'))throw new Error('The native Yoast request already contains a guard token');return {...fields.data,mcp_guard_token:leaseToken};}throw new Error('The native Yoast request data format is unsupported');};
  jqAjaxWrapper=function(){const callArgs=Array.from(arguments);const settings=typeof callArgs[0]==='string'?callArgs[1]:callArgs[0];const fields=readFields(settings?.data);const action=fields?.action; if(action!=='wpseo_elementor_save')return Reflect.apply(originalJqAjax,this,callArgs);
   let requestUrlMatches=false;try{requestUrlMatches=new URL(settings?.url||callArgs[0],location.href).href===nativeYoast?.url;}catch(error){}
   if(yoastIntercepted||!nativeCommandStarted||!nativeYoast||!nativeYoastContextValid()||elementor?.documents?.getCurrent?.()!==document||Number(document.id)!==args.pageId||!fields||fields.post_id!==String(args.pageId)||fields.nonce!==nativeYoast.nonce||!requestUrlMatches){yoastFailure=new Error('The native Yoast save request did not match the checked form and active document');yoastFailureKind='mismatch';const rejected=jQuery.Deferred();rejected.reject(yoastFailure);return rejected.promise();}
   yoastIntercepted=true;try{const modified={...settings,data:attachToken(fields)};if(typeof callArgs[0]==='string')callArgs[1]=modified;else callArgs[0]=modified;const request=Reflect.apply(originalJqAjax,this,callArgs);request.done(function(body,status,xhr){if(xhr?.status===200&&body?.success===true)yoastGate.resolveWith(this,arguments);else{yoastFailure=new Error('The native Yoast response did not confirm HTTP 200 JSON success');yoastFailureKind='response';yoastGate.rejectWith(this,[yoastFailure]);}}).fail(function(){yoastFailure=new Error('The native Yoast save request failed');yoastFailureKind='transport';yoastGate.rejectWith(this,[yoastFailure]);});return request;}catch(error){yoastFailure=error;yoastGate.reject(error);const rejected=jQuery.Deferred();rejected.reject(error);return rejected.promise();}};
  jQuery.ajax=jqAjaxWrapper;
 }
 addRequestWrapper=function(action,options){
  if(action!=='save_builder')return Reflect.apply(originalAddRequest,this,arguments);
  if(!nativeCommandStarted)return rejectNative(this,options,new Error('An unsolicited native save was refused during the checked operation'));
  if(elementor?.documents?.getCurrent?.()!==document||Number(document.id)!==args.pageId||elementor?.getContainer?.(args.elementId)!==element||!targetBelongsToDocument())return rejectNative(this,options,new Error('The native save target changed before dispatch'));
  if(intercepted)return rejectNative(this,options,new Error('A second native save was refused during the checked operation'));
  intercepted=true;
  const context=this,requestArgs=Array.from(arguments);
 (async()=>{
   if(saveComponent.startAutoSave!==fencedStartAutoSave||ajax.addRequest!==addRequestWrapper)throw new Error('The checked autosave or native save fence changed during the operation');
   const data=options?.data;
   if(!data||data.status!==beforeStatus||!Array.isArray(data.elements)||!data.settings||typeof data.settings!=='object')throw new Error('Native save payload did not match the checked document contract');
   if(elementor?.documents?.getCurrent?.()!==document||Number(document.id)!==args.pageId||elementor?.getContainer?.(args.elementId)!==element||!targetBelongsToDocument()||document.editor.isSaving!==true||document.editor.isChanged!==true)throw new Error('The editor document or save state changed before lease request');
   const lease=await boundedApi({path:'/mcp-for-page-builders/v1/document/'+args.pageId+'/editor-lease',method:'POST',data:{expected_revision:args.expectedRevision,operation_id:args.operationId,status:data.status,elements:data.elements,settings:data.settings,...(nativeYoast?{native_post_save:'yoast'}:{})}});
   leaseCreated=true;
   leaseToken=lease?.guard_token;
   if(lease?.post_id!==args.pageId||lease?.user_id!==user.id||lease?.expected_revision!==args.expectedRevision||lease?.operation_id!==args.operationId||typeof leaseToken!=='string'||!leaseToken||elementor?.documents?.getCurrent?.()!==document||Number(document.id)!==args.pageId||elementor?.getContainer?.(args.elementId)!==element||!targetBelongsToDocument()||ajax.addRequest!==addRequestWrapper||!same(payload(beforeStatus),editedPayload)||!same({status:data.status,elements:data.elements,settings:data.settings},editedPayload))throw new Error('The checked editor lease, payload, or save target did not match this document, user, revision, and operation');
   data.mcp_guard_token=leaseToken;
   if(ajax.addRequest!==addRequestWrapper)throw new Error('The native save transport changed before dispatch');
   ajax.addRequest=originalAddRequest;
   dispatched=true;
   let request;
   try{request=Reflect.apply(originalAddRequest,context,requestArgs);}catch(error){notifyNativeError(context,options,error);throw error;}
   request.done(function(){gate.resolveWith(this,arguments);}).fail(function(){gate.rejectWith(this,arguments);});
  })().catch(async(error)=>{
   if(!dispatched&&ajax.addRequest===originalAddRequest)ajax.addRequest=originalAddRequest;
   let releaseFailure=null;
   if(leaseCreated&&leaseToken&&!dispatched){
    try{await boundedApi({path:'/mcp-for-page-builders/v1/document/'+args.pageId+'/editor-lease',method:'DELETE',data:{guard_token:leaseToken}});}catch(releaseError){releaseFailure=message(releaseError);}
   }
  let failure=releaseFailure?message(error)+'; lease release failed: '+releaseFailure:message(error);
  if(!dispatched)failure=notifyNativeError(context,requestArgs[1],failure);
  gate.reject(new Error(failure));
  });
  return gate.promise();
 };
 ajax.addRequest=addRequestWrapper;
 let saveError=null;
 nativeCommandStarted=true;
   try{let timer;await Promise.race([$e.run('document/save/update',{status:beforeStatus,document}),new Promise((_,reject)=>{timer=setTimeout(()=>reject(new Error('Native save wait timed out; query the operation receipt before retrying')),30000);})]).finally(()=>clearTimeout(timer));}catch(error){saveError=error;}
 if(!intercepted)throw new Error('Elementor did not call the guarded save transport');
 if(saveError){
  if(dispatched){
   try{
    if(nativeYoast&&(!yoastIntercepted||yoastFailureKind==='response'||yoastFailureKind==='mismatch'))throw yoastFailure||new Error('The native Yoast save was not confirmed');
    const receipt=await boundedApi({path:'/mcp-for-page-builders/v1/document/'+args.pageId+'/editor-operation/'+encodeURIComponent(args.operationId)});
    if(receipt?.status==='saved'&&receipt?.readback_verified===true&&typeof receipt?.document_revision==='string'&&/^[a-fA-F0-9]{64}$/.test(receipt?.after_fingerprint||''))return result({status:'saved',readback_verified:true,document_revision:receipt.document_revision,operation_id:args.operationId,post_id:args.pageId,snapshot_kind:'native_editor',expected_state_fingerprint:receipt.after_fingerprint,restore_operation_id:receipt.restore_operation_id||null});
    if(typeof receipt?.status==='string')return result({error:'Native save receipt status: '+receipt.status,operation_id:args.operationId,post_id:args.pageId,snapshot_kind:'native_editor',expected_state_fingerprint:/^[a-fA-F0-9]{64}$/.test(receipt?.after_fingerprint||'')?receipt.after_fingerprint:null,restore_state:/^[a-fA-F0-9]{64}$/.test(receipt?.after_fingerprint||'')?'fingerprint_available':'indeterminate',restore_operation_id:receipt?.restore_operation_id||null,local_state:'native_receipt_failure_autosave_paused'});
    throw new Error('The native save receipt is incomplete');
   }catch(receiptError){throw new Error('Native save outcome is indeterminate; query this operation receipt before retrying: '+message(receiptError));}
  }
  throw saveError;
 }
 if(nativeYoast){if(!yoastIntercepted)throw yoastFailure||new Error('The native Yoast save hook did not run');let timer;await Promise.race([yoastGate.promise(),new Promise((_,reject)=>{timer=setTimeout(()=>reject(new Error('Native Yoast save wait timed out; query the operation receipt before retrying')),30000);})]).finally(()=>clearTimeout(timer));}
 const receipt=await boundedApi({path:'/mcp-for-page-builders/v1/document/'+args.pageId+'/editor-operation/'+encodeURIComponent(args.operationId)});
 if(receipt?.operation_id!==args.operationId||receipt?.status!=='saved'||receipt?.readback_verified!==true||typeof receipt?.document_revision!=='string'||!/^[a-fA-F0-9]{64}$/.test(receipt?.after_fingerprint||''))throw new Error('Native save receipt did not verify the saved document');
 return result({status:'saved',readback_verified:true,document_revision:receipt.document_revision,operation_id:args.operationId,post_id:args.pageId,snapshot_kind:'native_editor',expected_state_fingerprint:receipt.after_fingerprint,restore_operation_id:receipt.restore_operation_id||null});
}catch(error){
 if(changed&&!dispatched){
  try{
   if(!same(payload(beforeStatus),editedPayload)){saveComponent.stopAutoSave(document);return result({error:message(error),operation_id:args.operationId,post_id:args.pageId,local_state:'foreign_editor_change_autosave_paused'});}
   if(previousHad)await $e.run('document/elements/settings',{container:element,settings:{[args.key]:previousValue}});
   else element.settings.unset(args.key);
   const restoredPayload=payload(beforeStatus);
   const restored=same(restoredPayload,before)&&!document.editor.isSaving;
   if(restored)await $e.internal('document/save/set-is-modified',{status:false,document});
   const clean=restored&&document.editor.isChanged===false&&same(payload(beforeStatus),before);
   saveComponent.stopAutoSave(document);
   if(!clean)return result({error:message(error),operation_id:args.operationId,post_id:args.pageId,local_state:'restore_unverified_autosave_paused'});
  }catch(restoreError){saveComponent.stopAutoSave(document);return result({error:message(error),operation_id:args.operationId,post_id:args.pageId,restore_error:message(restoreError),local_state:'restore_failed_autosave_paused'});}
 }
 if(dispatched)saveComponent.stopAutoSave(document);
 let failure=message(error);
 if(dispatched&&nativeYoast){
  try{
   const receipt=await boundedApi({path:'/mcp-for-page-builders/v1/document/'+args.pageId+'/editor-operation/'+encodeURIComponent(args.operationId)});
   const fingerprint=/^[a-fA-F0-9]{64}$/.test(receipt?.after_fingerprint||'')?receipt.after_fingerprint:null;
   if(receipt?.status==='saved'&&receipt?.readback_verified===true&&typeof receipt?.document_revision==='string'&&fingerprint&&yoastFailureKind!=='response'&&yoastFailureKind!=='mismatch')return result({status:'saved',readback_verified:true,document_revision:receipt.document_revision,operation_id:args.operationId,post_id:args.pageId,snapshot_kind:'native_editor',expected_state_fingerprint:fingerprint,restore_operation_id:receipt.restore_operation_id||null});
   if(typeof receipt?.status==='string')return result({error:failure,operation_id:args.operationId,post_id:args.pageId,snapshot_kind:'native_editor',expected_state_fingerprint:fingerprint,restore_state:fingerprint?'fingerprint_available':'indeterminate',restore_operation_id:receipt.restore_operation_id||null,local_state:'native_receipt_failure_autosave_paused'});
  }catch(receiptError){failure+='; receipt read failed: '+message(receiptError);}
 }
 return result({error:failure,operation_id:args.operationId,post_id:args.pageId,local_state:dispatched?'save_outcome_indeterminate_autosave_paused':changed?'restored':'unchanged'});
}finally{
 if(originalAddRequest&&elementorCommon?.ajax?.addRequest===addRequestWrapper)elementorCommon.ajax.addRequest=originalAddRequest;
 if(originalJqAjax&&jQuery.ajax===jqAjaxWrapper)jQuery.ajax=originalJqAjax;
 if(saveComponent.startAutoSave===fencedStartAutoSave)saveComponent.startAutoSave=originalStartAutoSave;
 if(autosaveFenceInstalled&&!document.editor.isChanged)saveComponent.stopAutoSave(document);
}
})()"#.replace("__ARGS__", &args))
}

fn valid_editor_operation_id(value: &str) -> bool {
    (8..=128).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn valid_elementor_control_key(value: &str) -> bool {
    !value.is_empty() && !matches!(value, "__proto__" | "constructor" | "prototype")
}

fn js_get_preview_box(selector: &str) -> String {
    let selector =
        serde_json::to_string(selector).expect("CSS selectors serialize as JSON strings");
    format!(
        r"(()=>{{const iframe=document.querySelector('#elementor-preview-iframe');if(!iframe)return JSON.stringify({{error:'no preview iframe'}});const doc=iframe.contentDocument;const el=doc.querySelector({selector});if(!el)return JSON.stringify({{error:'not found'}});const b=el.getBoundingClientRect();const s=iframe.contentWindow.getComputedStyle(el);return JSON.stringify({{box:{{x:Math.round(b.x),y:Math.round(b.y),w:Math.round(b.width),h:Math.round(b.height)}},styles:{{'background-color':s.backgroundColor,color:s.color,'font-size':s.fontSize,padding:s.padding,margin:s.margin}},text:el.textContent?.trim()?.substring(0,200)||''}})}})()"
    )
}

#[cfg(test)]
fn normalize_empty_interactions(element: &mut Value) -> Result<()> {
    let object = element
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("Elementor document element must be an object"))?;
    if let Some(interactions) = object.get("interactions") {
        anyhow::ensure!(
            interactions
                .as_object()
                .is_some_and(serde_json::Map::is_empty),
            "Native Elementor save does not persist nonempty top-level interactions; refusing to save"
        );
        object.remove("interactions");
    }
    if object.get("elType").and_then(Value::as_str) == Some("widget")
        && object.get("isInner") == Some(&Value::Bool(false))
    {
        object.remove("isInner");
    }
    if object.get("isLocked") == Some(&Value::Bool(false)) {
        object.remove("isLocked");
    }
    if let Some(children) = object.get_mut("elements") {
        for child in children
            .as_array_mut()
            .ok_or_else(|| anyhow::anyhow!("Elementor child elements must be an array"))?
        {
            normalize_empty_interactions(child)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        js_checked_setting_save, normalize_empty_interactions, valid_editor_operation_id,
        valid_elementor_control_key,
    };
    use serde_json::json;

    #[test]
    fn native_defaults_are_removed_without_dropping_other_fields() {
        let mut element = json!({
            "id":"test1234",
            "elType":"widget",
            "isInner":false,
            "isLocked":false,
            "interactions":{},
            "custom_payload":{"preserved":true},
            "settings":{"title_color":"#123456","_css_classes":"hero"},
            "elements":[]
        });
        normalize_empty_interactions(&mut element).unwrap();
        assert_eq!(element["custom_payload"]["preserved"], json!(true));
        assert_eq!(element["settings"]["title_color"], json!("#123456"));
        assert_eq!(element["settings"]["_css_classes"], json!("hero"));
        assert!(element.get("interactions").is_none());
        assert!(element.get("isInner").is_none());
        assert!(element.get("isLocked").is_none());
    }

    #[test]
    fn native_true_lock_is_preserved() {
        let mut element = json!({"id":"test1234","isLocked":true});
        normalize_empty_interactions(&mut element).unwrap();
        assert_eq!(element["isLocked"], json!(true));
    }

    #[test]
    fn nonempty_interactions_are_preserved_and_rejected() {
        let mut element = json!({"id":"test1234","interactions":{"click":[]}});
        let error = normalize_empty_interactions(&mut element).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("does not persist nonempty top-level interactions")
        );
        assert_eq!(element["interactions"]["click"], json!([]));
    }

    #[test]
    fn checked_editor_operation_ids_use_the_bridge_character_and_length_contract() {
        assert!(valid_editor_operation_id("op_12345"));
        assert!(valid_editor_operation_id(&"a".repeat(128)));
        assert!(!valid_editor_operation_id("short"));
        assert!(!valid_editor_operation_id(&"a".repeat(129)));
        assert!(!valid_editor_operation_id("op/12345"));
    }

    #[test]
    fn checked_editor_rejects_empty_and_prototype_setting_keys() {
        for key in ["", "__proto__", "constructor", "prototype"] {
            assert!(!valid_elementor_control_key(key), "accepted {key:?}");
        }
        assert!(valid_elementor_control_key("title_color"));
        assert!(valid_elementor_control_key("_custom_control"));
    }

    #[test]
    fn checked_editor_binds_native_yoast_save_before_reading_the_final_receipt() {
        let script = js_checked_setting_save(
            42,
            "test1234",
            "title_color",
            &json!("#123456"),
            &"a".repeat(64),
            "operation_123",
            7,
        )
        .unwrap();
        let form_check = script.find("#yoast-form").unwrap();
        let local_edit = script.find("$e.run('document/elements/settings'").unwrap();
        let lease_mode = script.find("native_post_save:'yoast'").unwrap();
        let token_append = script.find("data.mcp_guard_token=leaseToken").unwrap();
        let hook_wait = script.find("yoastGate.promise()").unwrap();
        let final_receipt = script.rfind("/editor-operation/").unwrap();
        assert!(form_check < local_edit);
        assert!(lease_mode < token_append);
        assert!(token_append < hook_wait);
        assert!(hook_wait < final_receipt);
        assert!(script.contains("yoastForm.getAttribute('action')"));
        assert!(script.contains("nativeYoast.form.getAttribute('action')"));
        assert!(script.contains("xhr?.status===200&&body?.success===true"));
        assert!(script.contains("Reflect.apply(originalJqAjax,this,callArgs)"));
        assert!(script.contains("jQuery.ajax===jqAjaxWrapper)jQuery.ajax=originalJqAjax"));
    }

    #[test]
    fn checked_editor_script_checks_native_control_ownership_before_mutation() {
        let script = js_checked_setting_save(
            42,
            "test1234",
            "title_color",
            &json!("#123456"),
            &"a".repeat(64),
            "operation_123",
            7,
        )
        .unwrap();
        let capability_check = script
            .find("Object.prototype.hasOwnProperty.call(controls,args.key)")
            .unwrap();
        let local_edit = script.find("$e.run('document/elements/settings'").unwrap();
        let lease = script.find("/editor-lease").unwrap();
        assert!(capability_check < local_edit);
        assert!(capability_check < lease);
        assert!(script.contains("elementor?.getElementControls"));
    }
}

const fn js_wait_for_elementor_ready() -> &'static str {
    r"new Promise((resolve)=>{const check=()=>{if(window.elementor&&elementor.loaded)resolve('ready');else setTimeout(check,500)};check();setTimeout(()=>resolve('timeout'),30000)})"
}

#[async_trait]
impl Tool for ElementorEditor {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "elementor_editor",
            description: "Control the Elementor editor via CDP. Call 'open' first, then use select_widget or get_preview_box. Persist a setting only with set_setting_and_save, which requires expected_revision and operation_id and verifies the native save receipt. Requires an authenticated WordPress session and Elementor Core 4.2.3.",
            input_schema: json!({
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": { "type": "string", "enum": ["open", "select_widget", "set_setting", "set_setting_and_save", "get_preview_box", "save"], "description": "Editor action to perform" },
                    "page_id": { "type": "integer", "description": "Page whose editor tab is targeted by this action" },
                    "element_id": { "type": "string", "description": "8-char Elementor element ID. Required for 'select_widget' and 'set_setting'" },
                    "key": { "type": "string", "description": "Setting key for 'set_setting'" },
                    "value": { "description": "Setting value for 'set_setting'" },
                    "expected_revision": { "type": "string", "description": "64-character revision from the document read; required for set_setting_and_save" },
                    "operation_id": { "type": "string", "description": "Stable 8-128 character operation ID; required for set_setting_and_save and receipt lookup" },
                    "selector": { "type": "string", "description": "CSS selector for 'get_preview_box'" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let action = args["action"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("action required"))?;

        match action {
            "open" => {
                let page_id = args["page_id"]
                    .as_u64()
                    .ok_or_else(|| anyhow::anyhow!("page_id required"))?;
                let url = editor_url(page_id, wp).await?;
                let page = cdp::open_page(&url, 1440, 900).await?;

                let status: String = page
                    .evaluate(js_wait_for_elementor_ready())
                    .await?
                    .into_value()?;
                if status == "timeout" {
                    let evidence: String = page
                        .evaluate(r#"JSON.stringify({url:location.href,title:document.title,readyState:document.readyState,loginForm:!!document.querySelector('#loginform'),elementorLoaded:!!(window.elementor&&elementor.loaded),responseStatus:performance.getEntriesByType('navigation')[0]?.responseStatus})"#)
                        .await?
                        .into_value()?;
                    anyhow::bail!(
                        "Elementor editor did not load within 30s. Browser evidence: {evidence}"
                    );
                }
                Ok(ToolResult::text(format!(
                    "Editor opened for page {page_id}"
                )))
            }
            "select_widget" => {
                let eid = args["element_id"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("element_id required"))?;
                let page = editor_page(
                    args["page_id"]
                        .as_u64()
                        .ok_or_else(|| anyhow::anyhow!("page_id required"))?,
                    wp,
                )
                .await?;
                let result: String = page.evaluate(js_select_widget(eid)).await?.into_value()?;
                if result == "not_found" {
                    anyhow::bail!("Element {eid} not found in editor");
                }
                Ok(ToolResult::text(format!("Selected widget {eid}")))
            }
            "set_setting" => {
                anyhow::bail!(
                    "Standalone set_setting is disabled because native autosave could persist it before a revision-checked save; use set_setting_and_save"
                )
            }
            "set_setting_and_save" => {
                let page_id = args["page_id"]
                    .as_u64()
                    .ok_or_else(|| anyhow::anyhow!("page_id required"))?;
                let eid = args["element_id"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("element_id required"))?;
                let key = args["key"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("key required"))?;
                anyhow::ensure!(
                    valid_elementor_control_key(key),
                    "key must be nonempty and cannot be a prototype property"
                );
                let value = args.get("value").ok_or_else(|| {
                    anyhow::anyhow!("value required; use null explicitly to clear a setting")
                })?;
                let expected_revision = args["expected_revision"]
                    .as_str()
                    .filter(|revision| {
                        revision.len() == 64
                            && revision.bytes().all(|byte| byte.is_ascii_hexdigit())
                    })
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "a 64-character expected_revision from the document read is required"
                        )
                    })?;
                let operation_id = args["operation_id"]
                    .as_str()
                    .filter(|operation_id| valid_editor_operation_id(operation_id))
                    .ok_or_else(|| anyhow::anyhow!("operation_id must contain 8-128 letters, digits, underscores, or hyphens"))?;
                let runtime = wp.get("mcp-for-page-builders/v1/runtime").await?;
                anyhow::ensure!(
                    runtime["elementor_core_version"].as_str() == Some("4.2.3"),
                    "Checked native editor saves require the verified Elementor Core 4.2.3 runtime"
                );
                let user = wp.get("wp/v2/users/me?context=edit").await?;
                let expected_user_id =
                    user["id"].as_u64().filter(|id| *id > 0).ok_or_else(|| {
                        anyhow::anyhow!("Configured WordPress user identity could not be verified")
                    })?;
                let page = editor_page(page_id, wp).await?;
                let script = js_checked_setting_save(
                    page_id,
                    eid,
                    key,
                    value,
                    expected_revision,
                    operation_id,
                    expected_user_id,
                )?;
                let response: String = page.evaluate(script).await?.into_value()?;
                let response: Value = serde_json::from_str(&response)
                    .context("Elementor checked-save transport returned invalid JSON")?;
                if let Some(error) = response["error"].as_str() {
                    let mut recovery = json!({
                        "error": error.chars().take(500).collect::<String>(),
                        "post_id": response["post_id"],
                        "operation_id": response["operation_id"],
                        "local_state": response["local_state"],
                        "restore_error": response["restore_error"],
                        "snapshot_kind": response["snapshot_kind"],
                        "expected_state_fingerprint": response["expected_state_fingerprint"],
                        "restore_state": response["restore_state"],
                        "restore_operation_id": response["restore_operation_id"],
                    });
                    if response.get("restore_operation_id").is_some() {
                        recovery["restore_operation_id"] = response["restore_operation_id"].clone();
                    }
                    return Ok(ToolResult::error(recovery.to_string()));
                }
                if response["status"] != "saved"
                    || response["readback_verified"] != true
                    || response["operation_id"] != operation_id
                    || response["snapshot_kind"] != "native_editor"
                    || !response["expected_state_fingerprint"]
                        .as_str()
                        .is_some_and(|fingerprint| {
                            fingerprint.len() == 64
                                && fingerprint.bytes().all(|byte| byte.is_ascii_hexdigit())
                        })
                {
                    return Ok(ToolResult::error(
                        json!({
                            "error": "Checked native editor save returned an incomplete receipt",
                            "post_id": page_id,
                            "operation_id": operation_id,
                            "local_state": "save_outcome_indeterminate_autosave_paused"
                        })
                        .to_string(),
                    ));
                }
                Ok(ToolResult::text(
                    json!({
                        "status": "saved",
                        "post_id": page_id,
                        "operation_id": operation_id,
                        "document_revision": response["document_revision"],
                        "readback_verified": true,
                        "snapshot_kind": response["snapshot_kind"],
                        "expected_state_fingerprint": response["expected_state_fingerprint"],
                        "restore_operation_id": response["restore_operation_id"]
                    })
                    .to_string(),
                ))
            }
            "get_preview_box" => {
                let selector = args["selector"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("selector required"))?;
                let page = editor_page(
                    args["page_id"]
                        .as_u64()
                        .ok_or_else(|| anyhow::anyhow!("page_id required"))?,
                    wp,
                )
                .await?;
                let result: String = page
                    .evaluate(js_get_preview_box(selector))
                    .await?
                    .into_value()?;
                let val: Value = serde_json::from_str(&result)?;
                if let Some(err) = val.get("error") {
                    anyhow::bail!("{}", err.as_str().unwrap_or("unknown"));
                }
                Ok(ToolResult::text(serde_json::to_string_pretty(&val)?))
            }
            "save" => {
                anyhow::bail!(
                    "Standalone save is disabled because it has no revision-bound edit lease; use set_setting_and_save"
                )
            }
            _ => anyhow::bail!("Unknown action: {action}"),
        }
    }
}
