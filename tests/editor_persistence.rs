use anyhow::{Context, Result, ensure};
use chromiumoxide::cdp::browser_protocol::network::CookieParam;
use mcp_for_page_builders::elementor::{self, Element};
use mcp_for_page_builders::mcp::ToolResult;
use mcp_for_page_builders::tools::{
    Tool, editor::ElementorEditor, elementor_plan_tools::RestoreElementorSnapshot,
};
use mcp_for_page_builders::{cdp, wp::WpClient};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::Write,
    process::{Command, Stdio},
    sync::OnceLock,
};

#[path = "native_test_config.rs"]
mod native_test_config;

fn config() -> &'static native_test_config::NativeTestConfig {
    static CONFIG: OnceLock<native_test_config::NativeTestConfig> = OnceLock::new();
    CONFIG.get_or_init(|| {
        native_test_config::NativeTestConfig::load(false)
            .expect("valid isolated native test configuration")
    })
}

fn is_tool_failure(result: &Result<ToolResult>) -> bool {
    result
        .as_ref()
        .map_or(true, |result| result.is_error == Some(true))
}

fn remove_native_editor_defaults(element: &mut Value) {
    let Some(object) = element.as_object_mut() else {
        panic!("native Elementor element is not an object")
    };
    if object
        .get("interactions")
        .and_then(Value::as_object)
        .is_some_and(serde_json::Map::is_empty)
    {
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
            .expect("Elementor children are an array")
        {
            remove_native_editor_defaults(child);
        }
    }
}

fn check_native_page_settings_readback(
    page_id: u64,
    config: &native_test_config::NativeTestConfig,
) {
    let script = r#"$user=get_user_by('login',getenv('WP_TEST_USER')); if(!$user||!user_can($user,'manage_options')){throw new RuntimeException('configured test user must have manage_options');} wp_set_current_user($user->ID); $id=(int)getenv('MCP_NATIVE_PAGE_ID'); $post=get_post($id); if(!$post){throw new RuntimeException('native settings fixture post missing');} $settings=['id'=>(string)$id,'post_title'=>$post->post_title,'post_status'=>$post->post_status,'template'=>get_post_meta($id,'_wp_page_template',true)?:'default','post_excerpt'=>$post->post_excerpt,'post_featured_image'=>(string)get_post_thumbnail_id($id),'menu_order'=>(string)$post->menu_order,'comment_status'=>$post->comment_status,'vendor_page_setting'=>['nested'=>['keep'=>true]]]; update_post_meta($id,'_elementor_page_settings',['vendor_page_setting'=>['nested'=>['keep'=>true]]]); $check=mcp_bridge_editor_guard_settings_expectation($id,$settings,true); if(is_wp_error($check)||!$check['ordinary_matches']||!$check['special_matches']){throw new RuntimeException('browser-style native page settings did not verify: '.(is_wp_error($check)?$check->get_error_code():implode(',',array_merge($check['ordinary_paths'],$check['special_paths']))));} $settings['vendor_page_setting']['nested']['keep']=false; $check=mcp_bridge_editor_guard_settings_expectation($id,$settings,true); if(is_wp_error($check)||$check['ordinary_matches']||$check['ordinary_paths']!==['$.vendor_page_setting.nested.keep']){throw new RuntimeException('unknown ordinary page setting mismatch was not reported at its property path');} echo 'PASS native page settings readback';"#;
    let output = Command::new("docker")
        .args(["exec", "-u", config.docker_user.as_str(), "-e"])
        .arg(format!("WP_TEST_USER={}", config.user))
        .args(["-e"])
        .arg(format!("MCP_NATIVE_PAGE_ID={page_id}"))
        .args([
            config.container.as_str(),
            "wp",
            config.wp_path_arg.as_str(),
            "--quiet",
            "eval",
            script,
        ])
        .output()
        .expect("run cloned WordPress native page settings readback check");
    assert!(
        output.status.success(),
        "native page settings readback check failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("PASS native page settings readback"),
        "native page settings readback check did not report its assertions"
    );
}

fn diagnose_native_state_drift(
    page_id: u64,
    config: &native_test_config::NativeTestConfig,
    operation_id: &str,
) -> Result<String> {
    let script = r#"$user=get_user_by('login',getenv('WP_TEST_USER')); if(!$user||!user_can($user,'manage_options')) throw new RuntimeException('configured test user must have manage_options'); wp_set_current_user($user->ID); $id=(int)getenv('MCP_NATIVE_PAGE_ID'); $operation=getenv('MCP_NATIVE_OPERATION_ID'); $receipt=get_option('_mcp_elementor_editor_operation_'.hash('sha256',$operation)); if(!is_array($receipt)||(int)($receipt['post_id']??0)!==$id||($receipt['operation_id']??null)!==$operation||!is_array($receipt['after']??null)) throw new RuntimeException('matching saved operation snapshot missing'); $current=mcp_bridge_editor_guard_full_state($id); if(is_wp_error($current)) throw new RuntimeException('current native snapshot unavailable'); $after=$receipt['after']; $safe=function($name){return is_string($name)&&preg_match('/^[A-Za-z0-9_.:-]{1,96}$/D',$name)?$name:'[non-name]';}; $posts=[]; foreach(array_unique(array_merge(array_keys($after['post']),array_keys($current['post']))) as $key) if(!array_key_exists($key,$after['post'])||!array_key_exists($key,$current['post'])||$after['post'][$key]!==$current['post'][$key]) $posts[]=$safe($key); $metaMap=function($rows){$out=[];foreach($rows as $row){$key=(string)$row['meta_key'];$out[$key][]=hash('sha256',(string)$row['meta_id']."\0".(string)$row['meta_value']);}return $out;}; $before_meta=$metaMap($after['meta']);$now_meta=$metaMap($current['meta']);$meta=[];foreach(array_unique(array_merge(array_keys($before_meta),array_keys($now_meta))) as $key) if(($before_meta[$key]??null)!==($now_meta[$key]??null)) $meta[]=$safe($key); sort($meta); $css_fields=[];foreach(['status','inline','path','bytes'] as $field) if(($after['css'][$field]??null)!==($current['css'][$field]??null)) $css_fields[]=$field; $css_hash=function($css){return hash('sha256',wp_json_encode([$css['status']??null,isset($css['inline'])?hash('sha256',(string)$css['inline']):null,isset($css['bytes'])?hash('sha256',(string)$css['bytes']):null]));}; $revision_hash=function($state){return hash('sha256',wp_json_encode([$state['revision_ids']??[], $state['revisions']??[]]));}; $revision_changed=!hash_equals($revision_hash($after),$revision_hash($current)); $categories=[];if($posts)$categories[]='post';if($meta)$categories[]='metadata';if($revision_changed)$categories[]='revisions';if($css_fields)$categories[]='css'; echo "MCP_NATIVE_DRIFT_V1\t".wp_json_encode(['operation_matches'=>true,'categories'=>$categories,'post_fields'=>array_slice($posts,0,40),'metadata_keys'=>array_slice($meta,0,40),'revision_family_changed'=>$revision_changed,'revision_count_before'=>count($after['revision_ids']??[]),'revision_count_now'=>count($current['revision_ids']??[]),'css_fields_changed'=>$css_fields,'css_content_hashes_match'=>hash_equals($css_hash($after['css']),$css_hash($current['css']))])."\n";"#;
    let output = Command::new("docker")
        .args(["exec", "-u", config.docker_user.as_str(), "-e"])
        .arg(format!("WP_TEST_USER={}", config.user))
        .args(["-e"])
        .arg(format!("MCP_NATIVE_PAGE_ID={page_id}"))
        .args(["-e"])
        .arg(format!("MCP_NATIVE_OPERATION_ID={operation_id}"))
        .args([
            config.container.as_str(),
            "wp",
            config.wp_path_arg.as_str(),
            "--quiet",
            "eval",
            script,
        ])
        .output()
        .context("run failure-only native state drift diagnostic")?;
    ensure!(
        output.status.success(),
        "failure-only native drift diagnostic command failed with status {:?}; stderr bytes={}, stderr warning lines={}",
        output.status.code(),
        output.stderr.len(),
        String::from_utf8_lossy(&output.stderr)
            .lines()
            .filter(|line| line.contains("Undefined array key"))
            .count()
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let reports: Vec<_> = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("MCP_NATIVE_DRIFT_V1\t"))
        .collect();
    ensure!(
        reports.len() == 1,
        "failure-only native drift diagnostic expected exactly one framed report, found {}; Core warning lines={}",
        reports.len(),
        stdout
            .lines()
            .filter(|line| line.contains("Undefined array key"))
            .count()
    );
    let report: Value = serde_json::from_str(reports[0])
        .context("failure-only native drift diagnostic framed report was invalid JSON")?;
    ensure!(
        report["operation_matches"] == true && report["categories"].is_array(),
        "failure-only native drift diagnostic did not return the expected snapshot report"
    );
    Ok(report.to_string())
}

#[tokio::test]
async fn native_editor_setting_survives_save_and_reload() {
    let config = config();
    let base_url = &config.url;
    let capability_check = Command::new("docker")
        .args(["exec", "-u", config.docker_user.as_str(), "-e"])
        .arg(format!("WP_TEST_USER={}", config.user))
        .args([config.container.as_str(), "wp", config.wp_path_arg.as_str(), "eval", "--quiet"])
        .arg("$user=get_user_by('login',getenv('WP_TEST_USER')); if(!$user||!user_can($user,'manage_options')) throw new RuntimeException('configured test user must have manage_options'); echo 'PASS configured test user';")
        .output()
        .expect("check configured native test user capability");
    assert!(
        capability_check.status.success(),
        "configured native test user must have manage_options"
    );
    let wp = WpClient::new(&config.url, &config.user, &config.password);
    let browser = cdp::browser()
        .await
        .expect("isolated CDP browser should launch");
    let element_id = elementor::generate_id();
    let element = Element {
        id: element_id.clone(),
        el_type: "widget".into(),
        widget_type: Some("heading".into()),
        settings: json!({"title":"Before native editor save","header_size":"h2"}),
        elements: vec![],
        extra: HashMap::new(),
    };
    let created = wp.post("wp/v2/pages", &json!({
        "title": format!("IT native editor {element_id}"),
        "status": "draft",
        "template": "elementor_canvas",
        "meta": {"_elementor_data": elementor::serialize_data(&[element]).unwrap(), "_elementor_edit_mode":"builder"}
    })).await.expect("create isolated native editor draft");
    let page_id = created["id"]
        .as_u64()
        .expect("WordPress returned the draft page id");
    eprintln!("Created isolated native editor draft {page_id}");
    let new_title_color = "#123456";
    let mut session_token: Option<String> = None;

    let operation: Result<()> = async {
        check_native_page_settings_readback(page_id, config);
        let cookie_script = r#"$u=get_user_by('login',getenv('WP_TEST_USER')); if(!$u||!user_can($u,'manage_options')){throw new RuntimeException('configured test user must have manage_options');} $sessions=WP_Session_Tokens::get_instance($u->ID); $expiration=time()+900; $token=$sessions->create($expiration); try { $cookies=[SECURE_AUTH_COOKIE,wp_generate_auth_cookie($u->ID,$expiration,'secure_auth',$token),LOGGED_IN_COOKIE,wp_generate_auth_cookie($u->ID,$expiration,'logged_in',$token)]; echo wp_json_encode(['cookies'=>$cookies,'token'=>$token]); } catch(Throwable $error) { $sessions->destroy($token); throw $error; }"#;
        let cookie_output = Command::new("docker")
            .args(["exec", "-u", config.docker_user.as_str(), "-e"])
            .arg(format!("WP_TEST_USER={}", config.user))
            .args([config.container.as_str(), "wp", config.wp_path_arg.as_str(), "eval", "--quiet"])
            .arg(cookie_script)
            .output()
            .context("run configured WP-CLI to create the native session cookie")?;
        ensure!(cookie_output.status.success(), "configured WP-CLI could not create the native session cookie");
        let cookie_output = std::str::from_utf8(&cookie_output.stdout)
            .context("configured WP-CLI returned non-UTF8 cookie output")?;
        let cookie_data: Value = serde_json::from_str(cookie_output)
            .context("configured WP-CLI returned invalid native cookie data")?;
        session_token = Some(
            cookie_data["token"]
                .as_str()
                .context("native cookie data omitted its session token")?
                .to_owned(),
        );
        let cookie_lines = cookie_data["cookies"].as_array().context("native cookie data omitted the cookie pair")?;
        ensure!(cookie_lines.len() == 4, "configured WP-CLI did not return the expected native cookie pair");
        let cookie_lines = cookie_lines.iter().map(|value| value.as_str().context("native cookie entry is not text")).collect::<Result<Vec<_>>>()?;
        let cookies = [0, 2].map(|index| {
            CookieParam::builder()
                .name(cookie_lines[index])
                .value(cookie_lines[index + 1])
                .url(base_url)
                .path("/")
                .secure(base_url.starts_with("https://"))
                .build()
                .expect("native WordPress cookies satisfy CDP cookie requirements")
        });
        browser.set_cookies(cookies.to_vec()).await.context("set native session cookies in the isolated browser")?;

        let editor = ElementorEditor;
        editor.run(json!({"action":"open", "page_id":page_id}), &wp).await
            .context("native Elementor editor open failed")?;
        let runtime = wp.get("mcp-for-page-builders/v1/runtime").await?;
        let admin_url = runtime["admin_url"].as_str().context("runtime response has no admin_url")?;
        let editor_url = format!("{}/post.php?post={page_id}&action=elementor", admin_url.trim_end_matches('/'));
        let page = cdp::find_matching_page(&editor_url).await?
            .context("editor tab did not remain at its document URL")?;
        let state: String = page.evaluate(r#"JSON.stringify({
          currentModel: !!elementor.documents?.getCurrent?.()?.container?.model,
          currentElementCount: elementor.documents?.getCurrent?.()?.container?.model?.get('elements')?.length
        })"#).await?.into_value()?;
        let state: Value = serde_json::from_str(&state)?;
        ensure!(state["currentModel"].as_bool() == Some(true), "native editor document root unavailable: {state}");

        editor.run(json!({"action":"select_widget", "page_id":page_id, "element_id":element_id}), &wp).await
            .context("native editor could not select the test heading")?;
        let initial_document = elementor::get_page_document(&wp, page_id).await?;
        let expected_revision = initial_document.revision.clone();
        let entry_state: String = page.evaluate(format!(r#"(()=>{{const doc=elementor.documents.getCurrent();const target=elementor.getContainer({});return JSON.stringify({{hasSetting:Object.prototype.hasOwnProperty.call(target.settings.attributes,'title_color'),value:target.settings.attributes.title_color,payload:{{elements:doc.container.model.get('elements').toJSON({{remove:['default','editSettings','defaultEditSettings']}}),settings:doc.container.settings.toJSON({{remove:['default']}})}}}})}})()"#, serde_json::to_string(&element_id)?)).await?.into_value()?;
        let entry_state: Value = serde_json::from_str(&entry_state)?;
        eprintln!("Native editor entry title_color own_attribute={}", entry_state["hasSetting"]);
        let missing_value = editor.run(json!({
            "action":"set_setting_and_save", "page_id":page_id, "element_id":element_id,
            "key":"title_color", "expected_revision":expected_revision,
            "operation_id":format!("missing_{element_id}")
        }), &wp).await;
        ensure!(is_tool_failure(&missing_value), "checked editor mutation accepted a missing value argument");
        let mut stale_revision = expected_revision.clone();
        stale_revision.replace_range(0..1, if &stale_revision[..1] == "0" { "1" } else { "0" });
        let stale = editor.run(json!({
            "action":"set_setting_and_save", "page_id":page_id, "element_id":element_id,
            "key":"title_color", "value":new_title_color, "expected_revision":stale_revision,
            "operation_id":format!("stale_{element_id}")
        }), &wp).await;
        ensure!(is_tool_failure(&stale), "stale native editor revision was accepted");
        let after_stale = elementor::get_page_document(&wp, page_id).await?;
        ensure!(after_stale.revision == expected_revision, "stale editor operation changed the persisted document");

        let legacy_setting = editor.run(json!({
            "action":"set_setting", "page_id":page_id, "element_id":element_id,
            "key":"title_color", "value":new_title_color
        }), &wp).await;
        ensure!(is_tool_failure(&legacy_setting), "unchecked standalone setting mutation was accepted");

        let invalid_operation = editor.run(json!({
            "action":"set_setting_and_save", "page_id":page_id, "element_id":element_id,
            "key":"title_color", "value":new_title_color, "expected_revision":expected_revision,
            "operation_id":"short"
        }), &wp).await;
        ensure!(is_tool_failure(&invalid_operation), "invalid checked-save operation ID was accepted");

        let saving_state: String = page.evaluate(r#"(()=>{const doc=elementor.documents.getCurrent();doc.editor.isSaving=true;return String(doc.editor.isSaving)})()"#).await?.into_value()?;
        ensure!(saving_state == "true", "could not set the isolated in-flight-save precondition");
        let in_flight = editor.run(json!({
            "action":"set_setting_and_save", "page_id":page_id, "element_id":element_id,
            "key":"title_color", "value":new_title_color, "expected_revision":expected_revision,
            "operation_id":format!("busy_{element_id}")
        }), &wp).await;
        let reset_state: String = page.evaluate(r#"(()=>{const doc=elementor.documents.getCurrent();doc.editor.isSaving=false;return String(doc.editor.isSaving)})()"#).await?.into_value()?;
        ensure!(reset_state == "false", "could not clear the isolated in-flight-save precondition");
        ensure!(is_tool_failure(&in_flight), "checked editor mutation was accepted while a native save was in flight");

        let element_id_js = serde_json::to_string(&element_id)?;
        let install_failure: String = page.evaluate(format!(r#"(()=>{{window.__mcpTestElementId={element_id_js};const original=wp.apiFetch;window.__mcpOriginalApiFetch=original;const wrapper=function(options){{if(options?.path?.endsWith('/editor-lease')&&options?.method==='POST')return Promise.reject(new Error('pre-dispatch test rejection'));return Reflect.apply(original,this,arguments)}};window.__mcpTestApiFetch=wrapper;wp.apiFetch=wrapper;return 'installed'}})()"#)).await?.into_value()?;
        ensure!(install_failure == "installed", "could not install isolated pre-dispatch failure condition");
        let form_diagnostic: String = page.evaluate(format!(r#"(()=>{{const form=document.querySelector('#yoast-form');const action=form?.querySelector('input[name=action]')?.value;const post=form?.querySelector('input[name=post_id]')?.value;const nonce=form?.querySelector('input[name=_wpseo_elementor_nonce]');let formUrl=null,ajaxUrl=null;try{{formUrl=new URL(form?.getAttribute('action'),location.href)}}catch{{}}try{{ajaxUrl=new URL(window.ajaxurl,location.href)}}catch{{}}return JSON.stringify({{exists:!!form,method:form?.method,actionMatches:action==='wpseo_elementor_save',postMatches:post===String({page_id}),noncePresent:!!nonce?.value,sameOrigin:!!formUrl&&formUrl.origin===location.origin,ajaxUrlPresent:!!ajaxUrl,urlsMatch:!!formUrl&&!!ajaxUrl&&formUrl.href===ajaxUrl.href,formPath:formUrl?.pathname,ajaxPath:ajaxUrl?.pathname,activePost:Number(elementor?.documents?.getCurrent?.()?.id)}})}})()"#)).await?.into_value()?;
        let pre_dispatch = editor.run(json!({
            "action":"set_setting_and_save", "page_id":page_id, "element_id":element_id,
            "key":"title_color", "value":new_title_color, "expected_revision":expected_revision,
            "operation_id":format!("rollback_{element_id}")
        }), &wp).await;
        let restored_state: String = page.evaluate(format!(r#"(()=>{{if(wp.apiFetch===window.__mcpTestApiFetch)wp.apiFetch=window.__mcpOriginalApiFetch;const doc=elementor.documents.getCurrent();const saver=$e.components.get('document/save');const target=elementor.getContainer({});return JSON.stringify({{saving:doc.editor.isSaving,changed:doc.editor.isChanged,timer:!!saver.autoSaveTimers[doc.id],hasSetting:Object.prototype.hasOwnProperty.call(target.settings.attributes,'title_color'),value:target.settings.attributes.title_color,payload:{{elements:doc.container.model.get('elements').toJSON({{remove:['default','editSettings','defaultEditSettings']}}),settings:doc.container.settings.toJSON({{remove:['default']}})}}}})}})()"#, serde_json::to_string(&element_id)?)).await?.into_value()?;
        let restored_state: Value = serde_json::from_str(&restored_state)?;
        ensure!(is_tool_failure(&pre_dispatch), "checked save accepted a rejected pre-dispatch lease");
        let pre_dispatch_json = serde_json::to_value(pre_dispatch.as_ref().unwrap())?;
        let pre_dispatch_error = pre_dispatch_json["content"][0]["text"].as_str().context("pre-dispatch failure returned no structured error")?;
        ensure!(pre_dispatch_error.contains("pre-dispatch test rejection"), "injected pre-dispatch rejection was not the reported failure: {}; native form={form_diagnostic}", pre_dispatch_error.chars().take(300).collect::<String>());
        ensure!(restored_state["saving"] == false && restored_state["changed"] == false && restored_state["timer"] == false && restored_state["hasSetting"] == entry_state["hasSetting"] && restored_state["value"] == entry_state["value"] && restored_state["payload"] == entry_state["payload"], "pre-dispatch failure did not restore the exact native editor entry state: {restored_state}");
        let after_rollback = elementor::get_page_document(&wp, page_id).await?;
        ensure!(after_rollback.revision == expected_revision, "pre-dispatch failure changed the persisted document");

        let trace_install: String = page.evaluate(format!(r#"(()=>{{const postId={page_id};const namespace='.mcpYoastTrace';const read=function(data,url){{if(data instanceof FormData||data instanceof URLSearchParams){{return read(new URLSearchParams(data).toString(),url)}}if(typeof data==='string'){{const params=new URLSearchParams(data);let action=params.get('action');if(action==='elementor_ajax'){{let isSaveBuilder=false;const nested=params.get('actions');if(nested){{try{{isSaveBuilder=Object.prototype.hasOwnProperty.call(JSON.parse(nested),'save_builder')}}catch{{isSaveBuilder=nested.includes('save_builder')}}}}for(const key of params.keys())if(/^actions(?:\[|%5B)save_builder/i.test(key))isSaveBuilder=true;if(isSaveBuilder)action='elementor_ajax/save_builder'}}return {{action,post_id:params.get('post_id')||params.get('editor_post_id')}}}}if(Array.isArray(data)){{const params=new URLSearchParams();for(const entry of data)if(entry&&typeof entry.name==='string')params.append(entry.name,String(entry.value??''));return read(params.toString(),url)}}if(data&&typeof data==='object'){{let action=data.action;if(action==='elementor_ajax'&&(Object.prototype.hasOwnProperty.call(data.actions||{{}},'save_builder')||Object.keys(data).some(key=>/^actions\[save_builder\]/.test(key))))action='elementor_ajax/save_builder';return {{action,post_id:data.post_id||data.editor_post_id}}}}const query=new URL(url,location.href).searchParams;return {{action:query.get('action'),post_id:query.get('post_id')||query.get('editor_post_id')}}}};const matches=function(fields){{return (fields.action==='elementor_ajax/save_builder'||fields.action==='wpseo_elementor_save')&&String(fields.post_id||fields.editor_post_id||'')===String(postId)}};window.__mcpNativeSaveTrace=[];jQuery(document).off(namespace).on('ajaxSend'+namespace,function(_event,_xhr,settings){{const fields=read(settings.data,settings.url);if(matches(fields))window.__mcpNativeSaveTrace.push({{event:'send',action:fields.action,post_id:postId,at_ms:performance.now()}})}}).on('ajaxComplete'+namespace,function(_event,xhr,settings){{const fields=read(settings.data,settings.url);if(matches(fields))window.__mcpNativeSaveTrace.push({{event:'complete',action:fields.action,post_id:postId,at_ms:performance.now(),http_status:Number(xhr.status)||0,http_success:xhr.status>=200&&xhr.status<300,response_success:xhr.responseJSON?.success===true}})}});return 'installed'}})()"#)).await?.into_value()?;
        ensure!(trace_install == "installed", "could not install fixture-scoped native save trace");
        let operation_id = format!("native_{element_id}");
        let save_result = editor.run(json!({
            "action":"set_setting_and_save", "page_id":page_id, "element_id":element_id,
            "key":"title_color", "value":new_title_color, "expected_revision":expected_revision,
            "operation_id":operation_id
        }), &wp).await;
        let trace_json: String = page.evaluate(r#"(()=>{const trace=window.__mcpNativeSaveTrace||[];jQuery(document).off('.mcpYoastTrace');delete window.__mcpNativeSaveTrace;return JSON.stringify(trace)})()"#).await?.into_value()?;
        let native_save_trace: Value = serde_json::from_str(&trace_json)?;
        if is_tool_failure(&save_result) {
            let current = elementor::get_page_document(&wp, page_id).await?;
            let current_tree: Value = serde_json::from_str(&current.raw)?;
        let live: String = page.evaluate(r#"JSON.stringify(elementor.documents.getCurrent().container.model.get('elements').toJSON())"#).await?.into_value()?;
        let live: Value = serde_json::from_str(&live)?;
        let operation_id = save_result.as_ref().ok()
            .and_then(|result| serde_json::to_value(result).ok())
            .and_then(|result| result["content"][0]["text"].as_str().map(str::to_owned))
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .and_then(|error| error["operation_id"].as_str().map(str::to_owned));
        let state_drift_diagnostic = match operation_id.as_deref() {
            Some(operation_id) => diagnose_native_state_drift(page_id, config, operation_id)
                .map_err(|error| format!("diagnostic failed: {error:#}")),
            None => Err("diagnostic failed: failed save returned no operation ID".into()),
        };
        let state_drift_diagnostic = match state_drift_diagnostic {
            Ok(report) => report,
            Err(error) => error,
        };
        let receipt_diagnostic = if let Some(operation_id) = operation_id {
            match wp.get(&format!("mcp-for-page-builders/v1/document/{page_id}/editor-operation/{operation_id}")).await {
                Ok(receipt) => {
                    let error = receipt["error"].as_str().unwrap_or_default();
                    let path = ["ordinary page settings mismatch at ", "native page settings mismatch at "]
                        .iter().find_map(|prefix| error.split_once(prefix).map(|(_, path)| path.split(';').next().unwrap_or(path)));
                    json!({
                        "status": receipt["status"],
                        "readback_verified": receipt["readback_verified"],
                        "revision_valid": receipt["document_revision"].as_str().is_some_and(|value| value.len() == 64),
                        "fingerprints_match": receipt["after_fingerprint"].as_str().zip(receipt["current_state_fingerprint"].as_str()).is_some_and(|(saved, current)| saved == current),
                        "error_kind": if path.is_some() { "settings_path" } else if error.is_empty() { "none" } else { "other" },
                        "error_path": path,
                    })
                }
                Err(_) => json!({"query": "failed"}),
            }
        } else { json!({"query": "no_operation_id"}) };
        let tool_error = save_result.as_ref().ok()
                .and_then(|result| serde_json::to_value(result).ok())
                .and_then(|result| result["content"][0]["text"].as_str().map(str::to_owned))
                .and_then(|text| serde_json::from_str::<Value>(&text).ok()).map(|error| json!({
                "error": error["error"].as_str().map(|text| text.chars().take(300).collect::<String>()),
                "local_state": error["local_state"],
                "post_id": error["post_id"],
                "operation_id": error["operation_id"],
                "restore_operation_id": error["restore_operation_id"],
            })).unwrap_or_else(|| json!({"error":"non-JSON or missing structured tool error"}));
            anyhow::bail!("native editor save failed: {}; tool_error={tool_error}; receipt_diagnostic={receipt_diagnostic}; state_drift_diagnostic={state_drift_diagnostic}; live_setting_matches={}, persisted_setting_matches={}; native_save_trace={native_save_trace}", save_result.as_ref().err().map(|error| format!("{error:#}")).unwrap_or_else(|| "structured MCP error".into()), live[0]["settings"]["title_color"] == new_title_color, current_tree[0]["settings"]["title_color"] == new_title_color);
        }

        let trace = native_save_trace.as_array().context("native save trace is not an array")?;
        let event_index = |event: &str, action: &str| trace.iter().position(|entry| entry["event"] == event && entry["action"] == action);
        let elementor_complete = event_index("complete", "elementor_ajax/save_builder");
        ensure!(elementor_complete.is_some(), "native Elementor save completion was not observed; trace={native_save_trace}");
        let elementor_complete = elementor_complete.unwrap();
        let yoast_send = event_index("send", "wpseo_elementor_save");
        ensure!(yoast_send.is_some(), "native Yoast save request was not observed; trace={native_save_trace}");
        let yoast_send = yoast_send.unwrap();
        let yoast_complete = event_index("complete", "wpseo_elementor_save");
        ensure!(yoast_complete.is_some(), "native Yoast save response was not observed; trace={native_save_trace}");
        let yoast_complete = yoast_complete.unwrap();
        ensure!(elementor_complete < yoast_send && yoast_send < yoast_complete, "native Yoast save did not complete after the Elementor save: {native_save_trace}");
        ensure!(trace[yoast_complete]["http_status"] == 200 && trace[yoast_complete]["response_success"] == true, "native Yoast request did not return HTTP 200 JSON success: {native_save_trace}");

        let autosave_state: String = page.evaluate(r#"(()=>{const doc=elementor.documents.getCurrent();const saver=$e.components.get('document/save');return JSON.stringify({changed:doc.editor.isChanged,timer:!!saver.autoSaveTimers[doc.id]})})()"#).await?.into_value()?;
        let autosave_state: Value = serde_json::from_str(&autosave_state)?;
        ensure!(autosave_state["changed"] == false && autosave_state["timer"] == false, "successful checked save left native autosave scheduled: {autosave_state}");
        let live_tree: String = page.evaluate(r#"JSON.stringify(elementor.documents.getCurrent().container.model.get('elements').toJSON())"#).await?.into_value()?;
        let live_tree: Value = serde_json::from_str(&live_tree)?;
        ensure!(live_tree[0]["settings"]["title_color"] == new_title_color, "checked native editor action did not apply the requested setting");
        let saved_doc = elementor::get_page_document(&wp, page_id).await?;
        let saved_tree: Value = serde_json::from_str(&saved_doc.raw)?;
        ensure!(saved_tree[0]["settings"]["title_color"] == new_title_color, "REST readback did not contain the newly added style setting");
        let save_result_json = serde_json::to_value(save_result.as_ref().unwrap())?;
        let save_receipt: Value = serde_json::from_str(save_result_json["content"][0]["text"].as_str().context("checked save returned no receipt text")?)?;
        ensure!(save_receipt["snapshot_kind"] == "native_editor" && save_receipt["expected_state_fingerprint"].as_str().is_some_and(|fingerprint| fingerprint.len() == 64 && fingerprint.bytes().all(|byte| byte.is_ascii_hexdigit())), "native save receipt did not expose a validated restore fingerprint: {save_receipt}");
        let immediate_operation = wp.get(&format!("mcp-for-page-builders/v1/document/{page_id}/editor-operation/{operation_id}")).await?;
        ensure!(immediate_operation["status"] == "saved" && immediate_operation["readback_verified"] == true && immediate_operation["after_fingerprint"] == immediate_operation["current_state_fingerprint"] && immediate_operation["after_fingerprint"] == save_receipt["expected_state_fingerprint"], "Yoast completion did not produce an exact immediate full-state receipt: {immediate_operation}");
        page.reload().await?;
        let ready: String = page.evaluate(r#"new Promise((resolve)=>{const check=()=>{const model=window.elementor?.documents?.getCurrent?.()?.container?.model;if(window.elementor?.loaded&&model?.get('elements'))resolve('ready');else setTimeout(check,250)};check();setTimeout(()=>resolve('timeout'),30000)})"#).await?.into_value()?;
        ensure!(ready == "ready", "reloaded native Elementor editor did not become ready");
        let reloaded: String = page.evaluate(r#"JSON.stringify(elementor.documents.getCurrent().container.model.get('elements').toJSON({remove:['default','editSettings','defaultEditSettings']}))"#).await?.into_value()?;
        let reloaded: Value = serde_json::from_str(&reloaded)?;
        let mut reloaded = reloaded;
        for element in reloaded.as_array_mut().context("reloaded editor document is an array")? {
            remove_native_editor_defaults(element);
        }
        ensure!(reloaded == saved_tree, "native editor reload did not restore the exact saved Elementor tree");
        let final_document = elementor::get_page_document(&wp, page_id).await?;
        let final_operation = wp.get(&format!("mcp-for-page-builders/v1/document/{page_id}/editor-operation/{operation_id}")).await?;
        let final_state_drift = diagnose_native_state_drift(page_id, config, &operation_id).unwrap_or_else(|error| format!("diagnostic failed: {error:#}"));
        let final_state_drift: Value = serde_json::from_str(&final_state_drift).context("post-reload full-state drift diagnostic was not valid JSON")?;
        ensure!(final_operation["operation_id"] == operation_id && final_operation["status"] == "saved" && final_operation["readback_verified"] == false, "Yoast receipt did not preserve its saved state and report changed current state after reload: {final_operation}");
        ensure!(final_operation["after_fingerprint"] == immediate_operation["after_fingerprint"] && final_operation["current_state_fingerprint"] != final_operation["after_fingerprint"], "post-reload receipt did not preserve the exact save fingerprint and report current full-state drift: {final_operation}");
        ensure!(final_state_drift["categories"] == json!(["metadata"]) && final_state_drift["metadata_keys"] == json!(["_edit_lock"]) && final_state_drift["post_fields"] == json!([]) && final_state_drift["revision_family_changed"] == false && final_state_drift["css_fields_changed"] == json!([]) && final_state_drift["css_content_hashes_match"] == true, "post-reload state drift exceeded the observed editor lock metadata change: {final_state_drift}");
        ensure!(final_operation["document_revision"] == final_document.revision, "Yoast receipt document revision changed after editor reload");
        let stale_restore = RestoreElementorSnapshot.run(json!({"snapshot_kind":"native_editor","page_id":page_id,"operation_id":operation_id,"expected_state_fingerprint":immediate_operation["after_fingerprint"]}), &wp).await?;
        ensure!(stale_restore.is_error == Some(true), "stale native snapshot restore accepted the pre-reload full-state fingerprint");
        let after_stale_restore = wp.get(&format!("mcp-for-page-builders/v1/document/{page_id}/editor-operation/{operation_id}")).await?;
        ensure!(after_stale_restore["current_state_fingerprint"] == final_operation["current_state_fingerprint"] && after_stale_restore["readback_verified"] == false, "stale restore changed the post-reload full state");
        Ok(())
    }.await;

    let operation_error = operation.as_ref().err().map(|error| format!("{error:#}"));
    let mut cleanup_errors = Vec::new();
    if let Err(error) = browser.clear_cookies().await {
        cleanup_errors.push(format!("clear isolated CDP cookies: {error:#}"));
    }
    cdp::reset().await;
    if let Err(error) = wp
        .delete(&format!("wp/v2/pages/{page_id}?force=true"))
        .await
    {
        cleanup_errors.push(format!("delete exact test draft {page_id}: {error:#}"));
    }
    match wp.get(&format!("wp/v2/pages/{page_id}?context=edit")).await {
        Err(error)
            if error.to_string().contains("WP API 404")
                || error.to_string().contains("HTTP 404") => {}
        Err(error) => cleanup_errors.push(format!(
            "verify exact test draft {page_id} is absent: {error:#}"
        )),
        Ok(_) => cleanup_errors.push(format!(
            "exact test draft {page_id} still exists after cleanup"
        )),
    }
    let operation_id = format!("native_{element_id}");
    let cleanup_script = r#"$id=(int)getenv('MCP_NATIVE_PAGE_ID'); $operation=getenv('MCP_NATIVE_OPERATION_ID'); $receipt_key='_mcp_elementor_editor_operation_'.hash('sha256',$operation); $receipt=get_option($receipt_key); if(is_array($receipt)&&(int)($receipt['post_id']??0)===$id&&($receipt['operation_id']??null)===$operation) delete_option($receipt_key); $lease_key='_mcp_elementor_editor_lease_'.$id; $lease=get_option($lease_key); if(is_array($lease)&&($lease['operation_id']??null)===$operation) delete_option($lease_key); $page=get_post($id); $revisions=get_posts(['post_type'=>'revision','post_parent'=>$id,'post_status'=>'any','numberposts'=>-1,'fields'=>'ids']); if($page||$revisions||get_option($receipt_key)!==false||get_option($lease_key)!==false) throw new RuntimeException('exact fixture cleanup verification failed'); echo 'PASS exact native fixture cleanup';"#;
    let cleanup_output = Command::new("docker")
        .args(["exec", "-u", config.docker_user.as_str(), "-e"])
        .arg(format!("MCP_NATIVE_PAGE_ID={page_id}"))
        .args(["-e"])
        .arg(format!("MCP_NATIVE_OPERATION_ID={operation_id}"))
        .args([
            config.container.as_str(),
            "wp",
            config.wp_path_arg.as_str(),
            "--quiet",
            "eval",
            cleanup_script,
        ])
        .output();
    match cleanup_output {
        Ok(output)
            if output.status.success()
                && String::from_utf8_lossy(&output.stdout)
                    .contains("PASS exact native fixture cleanup") => {}
        Ok(output) => cleanup_errors.push(format!(
            "exact native receipt/lease/revision cleanup failed with status {:?}; stderr bytes={}",
            output.status.code(),
            output.stderr.len()
        )),
        Err(error) => cleanup_errors.push(format!(
            "could not run exact native receipt/lease/revision cleanup: {error}"
        )),
    }
    if let Some(token) = session_token {
        let session_cleanup: Result<_> = (|| {
            let mut child = Command::new("docker")
                .args(["exec", "-i", "-u", config.docker_user.as_str(), "-e"])
                .arg(format!("WP_TEST_USER={}", config.user))
                .args([
                    config.container.as_str(),
                    "wp",
                    config.wp_path_arg.as_str(),
                    "eval",
                    "--quiet",
                ])
                .arg("$user=get_user_by('login',getenv('WP_TEST_USER')); if(!$user||!user_can($user,'manage_options')) throw new RuntimeException('configured test user must have manage_options'); $token=stream_get_contents(STDIN); if(!is_string($token)||$token==='') throw new RuntimeException('owned session token missing'); $sessions=WP_Session_Tokens::get_instance($user->ID); $sessions->destroy($token); if($sessions->verify($token)!==false) throw new RuntimeException('owned session token cleanup failed'); echo 'PASS exact session cleanup';")
                .stdin(Stdio::piped())
                .spawn()
                .context("start exact native session cleanup")?;
            let mut stdin = child
                .stdin
                .take()
                .context("open exact native session cleanup stdin")?;
            let write_result = stdin
                .write_all(token.as_bytes())
                .context("write exact native session token to cleanup stdin");
            drop(stdin);
            let output = child
                .wait_with_output()
                .context("wait for exact native session cleanup")?;
            write_result?;
            Ok(output)
        })();
        match session_cleanup {
            Ok(output)
                if output.status.success()
                    && String::from_utf8_lossy(&output.stdout)
                        .contains("PASS exact session cleanup") => {}
            Ok(output) => cleanup_errors.push(format!(
                "exact native session cleanup failed with status {:?}; stderr bytes={}",
                output.status.code(),
                output.stderr.len()
            )),
            Err(error) => cleanup_errors.push(format!(
                "could not run exact native session cleanup: {error}"
            )),
        }
    }
    if !cleanup_errors.is_empty() {
        panic!(
            "native editor cleanup failed: {}; earlier test error: {}",
            cleanup_errors.join("; "),
            operation_error.as_deref().unwrap_or("none")
        );
    }
    operation.unwrap_or_else(|error| {
        panic!("native editor persistence failed for draft {page_id}: {error:#}")
    });
}
