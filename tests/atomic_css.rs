use anyhow::{Context, ensure};
use mcp_for_page_builders::{
    elementor::{ElementorService, get_page_document},
    tools::{
        Tool,
        css_map::AtomicCssConversion,
        elementor_plan_tools::{ApplyElementorPlan, RestoreElementorSnapshot},
        elementor_planning::{self, CssMigrationInput, PlanContext},
    },
    wp::WpClient,
};
use serde_json::{Value, json};
use std::{
    fs,
    net::TcpStream,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::OnceLock,
    time::{SystemTime, UNIX_EPOCH},
};
#[path = "native_test_config.rs"]
mod native_test_config;

fn config() -> &'static native_test_config::NativeTestConfig {
    static CONFIG: OnceLock<native_test_config::NativeTestConfig> = OnceLock::new();
    CONFIG.get_or_init(|| {
        native_test_config::NativeTestConfig::load(true)
            .expect("valid isolated native test configuration")
    })
}

const NATIVE_BOOTSTRAP: &str = r#"<?php
$uid=(int)getenv('MCP_TEST_USER_ID');
wp_set_current_user($uid);
if (!current_user_can('edit_posts')) throw new RuntimeException('test user cannot create pages');
$class='atomic-color-proof-'.getenv('MCP_FIXTURE_SUFFIX');
$node_id=getenv('MCP_NODE_ID');
$later=['id'=>'z-native-order','type'=>'class','label'=>'z-native-order','variants'=>[['meta'=>['breakpoint'=>'desktop','state'=>null],'props'=>['color'=>['$$type'=>'color','value'=>'#0000ff']]]]];
$earlier=['id'=>'a-native-order','type'=>'class','label'=>'a-native-order','variants'=>[['meta'=>['breakpoint'=>'desktop','state'=>null],'props'=>['color'=>['$$type'=>'color','value'=>'#ff0000']]]]];
$style=['id'=>$class,'type'=>'class','label'=>$class,'variants'=>[
  ['meta'=>['breakpoint'=>'desktop','state'=>null],'props'=>['background'=>['$$type'=>'background','value'=>['color'=>['$$type'=>'color','value'=>'#112233']]],'min-height'=>['$$type'=>'size','value'=>['size'=>50,'unit'=>'px']],'width'=>['$$type'=>'size','value'=>['size'=>320,'unit'=>'px']]]],
  ['meta'=>['breakpoint'=>'mobile','state'=>null],'props'=>['width'=>['$$type'=>'size','value'=>['size'=>280,'unit'=>'px']]]],
  ['meta'=>['breakpoint'=>'desktop','state'=>'hover'],'props'=>['opacity'=>['$$type'=>'size','value'=>['size'=>80,'unit'=>'%']]]],
]];
$parser=Elementor\Modules\AtomicWidgets\Parsers\Style_Parser::make(Elementor\Modules\AtomicWidgets\Styles\Style_Schema::get());
$parsed=$parser->parse($style);
if (!$parsed->is_valid()) throw new RuntimeException('native style parser rejected fixture: '.$parsed->errors()->to_string());
$node=['id'=>$node_id,'elType'=>'e-flexbox','settings'=>['classes'=>['$$type'=>'classes','value'=>[$class,'z-native-order','a-native-order']],'tag'=>['$$type'=>'string','value'=>'div']],'styles'=>[$parsed->unwrap(),$later,$earlier],'elements'=>[]];
$instance=Elementor\Plugin::instance()->elements_manager->create_element_instance($node);
if (!$instance || !is_callable([$instance,'get_data_for_save'])) throw new RuntimeException('native element serializer unavailable');
$canonical=$instance->get_data_for_save();
$id=(int)wp_insert_post(['post_type'=>'page','post_status'=>'publish','post_title'=>'Atomic color proof '.getenv('MCP_FIXTURE_SUFFIX'),'post_author'=>$uid,'post_content'=>''], true);
if (is_wp_error($id)) throw new RuntimeException($id->get_error_message());
try {
  update_post_meta($id,'_elementor_edit_mode','builder');
  update_post_meta($id,'_wp_page_template','elementor_canvas');
  $document=Elementor\Plugin::instance()->documents->get($id);
  if (!$document || !$document->save(['elements'=>[$canonical]])) throw new RuntimeException('native document save failed');
  $saved=get_post_meta($id,'_elementor_data',true);
  $tree=json_decode($saved,true,512,JSON_THROW_ON_ERROR);
  if (($tree[0]['settings']['classes']['value'][0]??null)!==$class) throw new RuntimeException('native document lost classes prop');
  if (($tree[0]['settings']['classes']['value'][1]??null)!=='z-native-order'||($tree[0]['settings']['classes']['value'][2]??null)!=='a-native-order') throw new RuntimeException('native document changed class binding order');
  if (array_column($tree[0]['styles'],'id')!==[$class,'z-native-order','a-native-order']) throw new RuntimeException('native document changed style definition order');
  if (($tree[0]['styles'][0]['variants'][0]['props']['background']['value']['color']['value']??null)!=='#112233') throw new RuntimeException('native document lost Atomic color style');
  if (!current_user_can('edit_post',$id)) throw new RuntimeException('test user cannot edit native fixture');
  file_put_contents(getenv('MCP_RESULT_PATH'),wp_json_encode(['page_id'=>$id,'class'=>$class,'node_id'=>$node_id,'document'=>$tree]));
} catch (Throwable $error) {
  wp_delete_post($id,true);
  throw $error;
}
echo 'native fixture saved';
"#;

const STATE_SCRIPT: &str = r#"<?php
global $wpdb;
$id=(int)getenv('MCP_PAGE_ID');
$out=['bridge'=>hash_file('sha256',WPMU_PLUGIN_DIR.'/mcp-for-page-builders/mcp-bridge.php'),'elementor'=>defined('ELEMENTOR_VERSION')?ELEMENTOR_VERSION:null,'pro'=>defined('ELEMENTOR_PRO_VERSION')?ELEMENTOR_PRO_VERSION:null,'protected'=>[],'atomic_cache'=>get_option('elementor_atomic_cache_validity__local'),'storedcss_print_method'=>get_option('elementor_css_print_method'),'artifacts'=>[],'fixture'=>null];
$operation_id=(string)getenv('MCP_OPERATION_ID');
if ($operation_id!=='') { $journal=get_option('_mcp_elementor_operation_'.hash('sha256',$operation_id)); $before_files=[]; foreach (($journal['before_state']['atomic_css']['files']??[]) as $name=>$file) $before_files[$name]=$file['sha256']??null; $before_atomic=$journal['before_state']['atomic_css']??[]; $out['operation_snapshot']=is_array($journal)?['status'=>$journal['status']??null,'has_before'=>is_array($journal['before_state']??null),'has_after'=>is_array($journal['after_state']??null),'has_after_fingerprint'=>is_string($journal['after_fingerprint']??null),'before_atomic_status'=>$before_atomic['status']??null,'before_cache_validity_hash'=>isset($before_atomic['cache_validity'])?hash('sha256',wp_json_encode($before_atomic['cache_validity'])):null,'before_files'=>$before_files,'restored'=>$journal['restored']??false]:null; }
foreach ([8416,22393] as $protected_id) {
  $post=get_post($protected_id,ARRAY_A);
  $meta=$wpdb->get_results($wpdb->prepare("SELECT meta_id,meta_key,meta_value FROM {$wpdb->postmeta} WHERE post_id=%d ORDER BY meta_id",$protected_id),ARRAY_A);
  $out['protected'][(string)$protected_id]=['post'=>$post?hash('sha256',wp_json_encode($post)):null,'meta'=>hash('sha256',wp_json_encode($meta))];
}
$active_kit=Elementor\Plugin::instance()->kits_manager->get_active_kit();
$kit_id=$active_kit?(int)$active_kit->get_id():0;
$kit_meta=$kit_id?$wpdb->get_results($wpdb->prepare("SELECT meta_id,meta_key,meta_value FROM {$wpdb->postmeta} WHERE post_id=%d ORDER BY meta_id",$kit_id),ARRAY_A):[];
$class_state=$active_kit?Elementor\Modules\GlobalClasses\Global_Classes_Repository::make($active_kit)->all(true)->get():[];
$out['kit']=['id'=>$kit_id,'post'=>$kit_id&&get_post($kit_id)?hash('sha256',wp_json_encode(get_post($kit_id,ARRAY_A))):null,'meta'=>hash('sha256',wp_json_encode($kit_meta)),'classes'=>hash('sha256',wp_json_encode($class_state))];
$dir=wp_upload_dir()['basedir'].'/elementor/css';
foreach (glob($dir.'/*')?:[] as $file) {
  if (!is_file($file)) continue;
  $name=basename($file);
  if (preg_match('/^(base-|post-(8416|22393)\\.css|local-(8416|22393)-)/',$name)) $out['artifacts'][$name]=hash_file('sha256',$file);
}
ksort($out['artifacts']);
if ($id>0) {
  $meta=$wpdb->get_results($wpdb->prepare("SELECT meta_id,meta_key,meta_value FROM {$wpdb->postmeta} WHERE post_id=%d ORDER BY meta_id",$id),ARRAY_A);
  $post=get_post($id,ARRAY_A);
  $files=[];
  foreach (glob($dir.'/local-'.$id.'-*')?:[] as $file) if (is_file($file)) $files[basename($file)]=hash_file('sha256',$file);
  foreach (glob($dir.'/post-'.$id.'.css')?:[] as $file) if (is_file($file)) $files[basename($file)]=hash_file('sha256',$file);
  ksort($files);
  $cache=$out['atomic_cache'];
  $style_class=(string)getenv('MCP_STYLE_CLASS');
  $style_rules=[];
  if ($style_class!=='') foreach (array_keys($files) as $name) { $path=$dir.'/'.$name; foreach (file($path,FILE_IGNORE_NEW_LINES)?:[] as $line) if (str_contains($line,$style_class)) $style_rules[$name][]=$line; }
  $out['fixture']=['post'=>$post?hash('sha256',wp_json_encode($post)):null,'document'=>json_decode((string)get_post_meta($id,'_elementor_data',true),true),'meta'=>hash('sha256',wp_json_encode($meta)),'cache_child'=>is_array($cache)?($cache['children'][(string)$id]??null):null,'files'=>$files,'style_rules'=>$style_rules];
}
file_put_contents(getenv('MCP_RESULT_PATH'),wp_json_encode($out));
echo 'state captured';
"#;

const CLEANUP_SCRIPT: &str = r#"<?php
$id=(int)getenv('MCP_PAGE_ID');
$ops=json_decode(getenv('MCP_OPERATION_IDS'),true,512,JSON_THROW_ON_ERROR);
foreach ($ops as $operation_id) {
  $hash=hash('sha256',$operation_id);
  delete_option('_mcp_elementor_operation_'.$hash);
  delete_option('_mcp_elementor_resources_'.$hash);
}
if ($id>0) {
  wp_delete_post($id,true);
  $cache=get_option('elementor_atomic_cache_validity__local');
  if (is_array($cache)&&isset($cache['children'])&&is_array($cache['children'])&&array_key_exists((string)$id,$cache['children'])) {
    unset($cache['children'][(string)$id]);
    update_option('elementor_atomic_cache_validity__local',$cache,false);
  }
  $dir=wp_upload_dir()['basedir'].'/elementor/css';
  foreach (array_merge(glob($dir.'/local-'.$id.'-*')?:[],glob($dir.'/post-'.$id.'.css')?:[]) as $file) if (is_file($file)) unlink($file);
}
$remaining=[];
foreach ($ops as $operation_id) {
  $hash=hash('sha256',$operation_id);
  if (get_option('_mcp_elementor_operation_'.$hash)!==false||get_option('_mcp_elementor_resources_'.$hash)!==false) $remaining[]=$operation_id;
}
if ($id>0&&get_post($id)) $remaining[]='fixture-page-remains';
if ($remaining) throw new RuntimeException('owned fixture cleanup incomplete');
file_put_contents(getenv('MCP_RESULT_PATH'),wp_json_encode(['clean'=>true]));
echo 'owned fixture removed';
"#;

const FOREIGN_CONFLICT_SCRIPT: &str = r#"<?php
global $wpdb;
$id=(int)getenv('MCP_PAGE_ID');
$action=(string)getenv('MCP_ACTION');
$meta_key='_mcp_foreign_restore_conflict_'.$id;
if ($action==='add_meta') add_post_meta($id,$meta_key,'foreign',true);
elseif ($action==='delete_meta') delete_post_meta($id,$meta_key);
elseif ($action==='foreign_title') $wpdb->update($wpdb->posts,['post_title'=>'Foreign title '.$id],['ID'=>$id]);
elseif ($action==='restore_title') $wpdb->update($wpdb->posts,['post_title'=>(string)getenv('MCP_ORIGINAL_TITLE')],['ID'=>$id]);
clean_post_cache($id);
file_put_contents(getenv('MCP_RESULT_PATH'),wp_json_encode(['title'=>get_post_field('post_title',$id),'foreign_meta'=>get_post_meta($id,$meta_key,true)]));
echo 'foreign-state fixture updated';
"#;

const ATOMIC_ARTIFACT_SCENARIOS: &str = r#"<?php
$uid=(int)getenv('MCP_TEST_USER_ID');
wp_set_current_user($uid);
if (!current_user_can('edit_posts')) throw new RuntimeException('test user cannot create empty-style fixture');
$id=(int)getenv('MCP_PAGE_ID');
$dir=wp_upload_dir()['basedir'].'/elementor/css';
$files=glob($dir.'/local-'.$id.'-frontend-*.css')?:[];
$artifact=null;
foreach ($files as $file) if (is_file($file)&&is_readable($file)&&filesize($file)>0) {$artifact=$file;break;}
if (!$artifact) throw new RuntimeException('nonempty native Atomic fixture artifact missing');
$bytes=file_get_contents($artifact);
$mode=fileperms($artifact)&0777;
if (!unlink($artifact)) throw new RuntimeException('could not remove owned Atomic fixture artifact');
$missing=mcp_bridge_atomic_css_artifact_state($id);
$leaf=basename($artifact,'.css');
$breakpoint=substr($leaf,strrpos($leaf,'-')+1);
$tree=get_option('elementor_atomic_cache_validity__local');
$valid_leaf=$tree['children'][(string)$id]['children']['frontend']['children'][$breakpoint]['state']??($tree['children'][(string)$id]['children']['frontend']['children'][$breakpoint]??null);
if (is_wp_error($missing)||$missing['active_context_status']!=='stale'||$valid_leaf!==true) {
  file_put_contents($artifact,$bytes);chmod($artifact,$mode);
  throw new RuntimeException('missing nonempty CSS was accepted while its native validity leaf stayed true');
}
if (file_put_contents($artifact,$bytes)!==strlen($bytes)||!chmod($artifact,$mode)) throw new RuntimeException('could not restore owned Atomic fixture artifact');
$original_cache=$tree;
unset($tree['children'][(string)$id]['children']['frontend']['children'][$breakpoint]);
if (!update_option('elementor_atomic_cache_validity__local',$tree,false)) throw new RuntimeException('could not remove the owned Atomic validity leaf');
try {
  $missing_leaf=mcp_bridge_atomic_css_artifact_state($id);
  if (is_wp_error($missing_leaf)||$missing_leaf['active_context_status']!=='stale') throw new RuntimeException('missing nonempty CSS validity leaf was accepted');
} finally {
  update_option('elementor_atomic_cache_validity__local',$original_cache,false);
}
$empty_id=wp_insert_post(['post_type'=>'page','post_status'=>'draft','post_title'=>'Atomic empty-style proof','post_content'=>''],true);
if (is_wp_error($empty_id)) throw new RuntimeException($empty_id->get_error_message());
try {
  update_post_meta($empty_id,'_elementor_edit_mode','builder');
  $element=['id'=>substr(md5((string)$empty_id),0,7),'elType'=>'e-flexbox','settings'=>['tag'=>['$$type'=>'string','value'=>'div']],'styles'=>[],'elements'=>[]];
  $instance=Elementor\Plugin::instance()->elements_manager->create_element_instance($element);
  if (!$instance||!is_callable([$instance,'get_data_for_save'])) throw new RuntimeException('empty Atomic fixture serializer unavailable');
  $document=Elementor\Plugin::instance()->documents->get($empty_id);
  if (!$document||!$document->save(['elements'=>[$instance->get_data_for_save()]])) throw new RuntimeException('empty Atomic document save failed');
  $expected=mcp_bridge_atomic_css_expected_css($empty_id);
  if (is_wp_error($expected)||array_filter($expected,static fn($css)=>$css!=='')) throw new RuntimeException('empty Atomic fixture produced unexpected CSS');
  $regenerated=mcp_bridge_regenerate_css($empty_id);
  if (is_wp_error($regenerated)) throw new RuntimeException('legitimately empty Atomic styles failed regeneration: '.$regenerated->get_error_message());
} finally {
  wp_delete_post($empty_id,true);
  $cache=get_option('elementor_atomic_cache_validity__local');
  if (is_array($cache)&&isset($cache['children'])&&is_array($cache['children'])) {
    unset($cache['children'][(string)$empty_id]);
    update_option('elementor_atomic_cache_validity__local',$cache,false);
  }
  foreach (array_merge(glob($dir.'/local-'.$empty_id.'-*')?:[],glob($dir.'/post-'.$empty_id.'.css')?:[]) as $file) if (is_file($file)) unlink($file);
}
file_put_contents(getenv('MCP_RESULT_PATH'),wp_json_encode(['missing_status'=>$missing['status'],'missing_leaf_status'=>$missing_leaf['active_context_status']??null,'empty_styles_regenerated'=>true]));
echo 'Atomic artifact scenarios passed';
"#;

const NODE_CDP: &str = r#"
const [mode,pageUrl,className,port]=process.argv.slice(1);
const cdp=`http://127.0.0.1:${port}`;
const sleep=ms=>new Promise(resolve=>setTimeout(resolve,ms));
async function json(url,init){let error;for(let i=0;i<40;i++){try{const response=await fetch(url,init);if(!response.ok)throw Error(`Chrome DevTools HTTP ${response.status}`);return response.json()}catch(e){error=e;await sleep(250)}}throw error}
let targets=await json(`${cdp}/json`);
let target;
if(mode==='new'||!targets.some(x=>x.type==='page'&&x.url.startsWith(pageUrl))) target=await json(`${cdp}/json/new?${encodeURIComponent(pageUrl)}`,{method:'PUT'});
else target=targets.find(x=>x.type==='page'&&x.url.startsWith(pageUrl));
const socket=new WebSocket(target.webSocketDebuggerUrl); await new Promise((resolve,reject)=>{socket.onopen=resolve;socket.onerror=reject});
let next=0;const pending=new Map();socket.onmessage=event=>{const message=JSON.parse(event.data);if(message.id&&pending.has(message.id)){pending.get(message.id)(message);pending.delete(message.id)}};
const call=(method,params={})=>new Promise((resolve,reject)=>{const id=++next;const timer=setTimeout(()=>{pending.delete(id);reject(Error(`CDP timed out: ${method}`))},15000);pending.set(id,m=>{clearTimeout(timer);m.error?reject(Error(m.error.message)):resolve(m.result)});socket.send(JSON.stringify({id,method,params}))});
await call('Page.enable');await call('Runtime.enable');
await call('Emulation.setDeviceMetricsOverride',{width:mode==='mobile'?375:1280,height:mode==='mobile'?812:900,deviceScaleFactor:1,mobile:mode==='mobile'});
if(mode==='reload') await call('Page.reload',{ignoreCache:false});
let state;
for(let attempt=0;attempt<40;attempt++){
 const evaluated=await call('Runtime.evaluate',{expression:`JSON.stringify((()=>{const e=document.querySelector('.'+${JSON.stringify(className)});return {ready:document.readyState,href:location.href,found:!!e,classes:e?[...e.classList]:[],background:e?getComputedStyle(e).backgroundColor:null,color:e?getComputedStyle(e).color:null,minHeight:e?getComputedStyle(e).minHeight:null,width:e?getComputedStyle(e).width:null,opacity:e?getComputedStyle(e).opacity:null,links:[...document.querySelectorAll('link[rel=stylesheet]')].map(x=>x.href).filter(x=>x.includes('/uploads/elementor/css/'))}})())`,returnByValue:true});
 state=JSON.parse(evaluated.result.value);if(state.ready==='complete'&&state.found)break;await sleep(250);
}
if(mode==='hover'&&state.found){const box=await call('Runtime.evaluate',{expression:`(()=>{const r=document.querySelector('.'+${JSON.stringify(className)}).getBoundingClientRect();return {x:r.left+r.width/2,y:r.top+r.height/2}})()`,returnByValue:true});await call('Input.dispatchMouseEvent',{type:'mouseMoved',x:box.result.value.x,y:box.result.value.y});await sleep(100);const evaluated=await call('Runtime.evaluate',{expression:`JSON.stringify((()=>{const e=document.querySelector('.'+${JSON.stringify(className)});return {ready:document.readyState,href:location.href,found:!!e,classes:e?[...e.classList]:[],background:e?getComputedStyle(e).backgroundColor:null,color:e?getComputedStyle(e).color:null,minHeight:e?getComputedStyle(e).minHeight:null,width:e?getComputedStyle(e).width:null,opacity:e?getComputedStyle(e).opacity:null,links:[...document.querySelectorAll('link[rel=stylesheet]')].map(x=>x.href).filter(x=>x.includes('/uploads/elementor/css/'))}})())`,returnByValue:true});state=JSON.parse(evaluated.result.value)}
if(mode==='mobile'&&state.found){const evaluated=await call('Runtime.evaluate',{expression:`JSON.stringify((()=>{const e=document.querySelector('.'+${JSON.stringify(className)});return {ready:document.readyState,href:location.href,found:!!e,classes:e?[...e.classList]:[],background:e?getComputedStyle(e).backgroundColor:null,color:e?getComputedStyle(e).color:null,minHeight:e?getComputedStyle(e).minHeight:null,width:e?getComputedStyle(e).width:null,opacity:e?getComputedStyle(e).opacity:null,links:[...document.querySelectorAll('link[rel=stylesheet]')].map(x=>x.href).filter(x=>x.includes('/uploads/elementor/css/'))}})())`,returnByValue:true});state=JSON.parse(evaluated.result.value)}
socket.close();console.log(JSON.stringify(state));
"#;

fn command(args: &[&str]) -> anyhow::Result<std::process::Output> {
    let output = Command::new(args[0]).args(&args[1..]).output()?;
    ensure!(
        output.status.success(),
        "{} failed: {}",
        args[0],
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(output)
}

fn php_result(source: &str, stem: &str, args: &[(&str, String)]) -> anyhow::Result<Value> {
    let nonce = format!("{}-{}", std::process::id(), unique_suffix());
    let local = std::env::temp_dir().join(format!("{stem}-{nonce}.php"));
    let remote = format!("/tmp/{stem}-{nonce}.php");
    let result_local = std::env::temp_dir().join(format!("{stem}-{nonce}.json"));
    let result_remote = format!("/tmp/{stem}-{nonce}.json");
    fs::write(&local, source)?;
    let local_text = local.to_str().context("temporary path is not UTF-8")?;
    let result_text = result_local
        .to_str()
        .context("temporary path is not UTF-8")?;
    let execution = (|| {
        command(&[
            "docker",
            "cp",
            local_text,
            &format!("{}:{remote}", config().container),
        ])?;
        let mut args_vec = vec![
            "docker",
            "exec",
            "-e",
            Box::leak(format!("MCP_RESULT_PATH={result_remote}").into_boxed_str()),
        ];
        for (name, value) in args {
            args_vec.push("-e");
            args_vec.push(Box::leak(format!("{name}={value}").into_boxed_str()));
        }
        args_vec.extend([
            "-u",
            config().docker_user.as_str(),
            config().container.as_str(),
            "wp",
            config().wp_path_arg.as_str(),
            "eval-file",
            &remote,
        ]);
        command(&args_vec)?;
        command(&[
            "docker",
            "cp",
            &format!("{}:{result_remote}", config().container),
            result_text,
        ])?;
        Ok(serde_json::from_slice(&fs::read(&result_local)?)?)
    })();
    let remote_cleanup = command(&[
        "docker",
        "exec",
        config().container.as_str(),
        "rm",
        "-f",
        &remote,
        &result_remote,
    ]);
    let _ = fs::remove_file(local);
    let _ = fs::remove_file(result_local);
    match (execution, remote_cleanup) {
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error.context("remove owned remote PHP helper files")),
        (Ok(value), Ok(_)) => Ok(value),
    }
}

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock must be after the Unix epoch")
        .as_nanos();
    format!("{:012x}", nanos & 0xffffffffffff)
}

fn tool_json(value: &impl serde::Serialize) -> anyhow::Result<Value> {
    let value = serde_json::to_value(value)?;
    Ok(serde_json::from_str(
        value["content"][0]["text"]
            .as_str()
            .context("tool response omitted JSON text")?,
    )?)
}

struct ChromeProcess {
    child: Child,
    profile: PathBuf,
}

impl ChromeProcess {
    fn start() -> anyhow::Result<Self> {
        let port = config()
            .cdp_port
            .expect("browser test configuration includes MCP_TEST_CDP_PORT");
        ensure!(
            TcpStream::connect(("127.0.0.1", port)).is_err(),
            "configured Chrome DevTools port is already in use"
        );
        let profile = std::env::temp_dir().join(format!("atomic-css-chrome-{}", unique_suffix()));
        let resolver_rule = format!(
            "--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE {}",
            config().host
        );
        let child = Command::new(
            config()
                .chrome
                .as_ref()
                .expect("browser config includes Chrome"),
        )
        .args([
            "--headless=new",
            "--remote-debugging-address=127.0.0.1",
            &format!("--remote-debugging-port={port}"),
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-background-networking",
            &resolver_rule,
            &format!("--user-data-dir={}", profile.display()),
            "about:blank",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("start isolated guest Chrome")?;
        Ok(Self { child, profile })
    }
}

impl Drop for ChromeProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.profile);
    }
}

fn browser_style(mode: &str, url: &str, class_name: &str) -> anyhow::Result<Value> {
    let output = Command::new("node")
        .arg("-e")
        .arg(NODE_CDP)
        .args([
            mode,
            url,
            class_name,
            &config().cdp_port.expect("CDP port").to_string(),
        ])
        .output()
        .context("run guest Chrome CDP inspection")?;
    ensure!(
        output.status.success(),
        "Chrome CDP inspection failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(serde_json::from_slice(&output.stdout)?)
}

async fn restore(
    wp: &WpClient,
    page_id: u64,
    operation_id: &str,
    document_snapshot: bool,
) -> anyhow::Result<()> {
    let current = get_page_document(wp, page_id).await?;
    let outcome = RestoreElementorSnapshot
        .run(
            json!({"page_id":page_id,"operation_id":operation_id,"expected_revision":current.revision}),
            wp,
        )
        .await?;
    ensure!(
        outcome.is_error != Some(true),
        "generic snapshot restore failed: {}",
        serde_json::to_string(&outcome)?
    );
    let restored = tool_json(&outcome)?;
    ensure!(
        restored["restored"] == true,
        "generic snapshot was not restored: {restored}"
    );
    if document_snapshot {
        ensure!(
            restored["document"]["cache_regenerated"] == true,
            "restored CSS cache was not regenerated: {restored}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn native_atomic_color_changes_in_guest_and_restores() -> anyhow::Result<()> {
    let config = config();
    let wp = WpClient::new(&config.url, &config.user, &config.password);
    let current_user = wp.get("wp/v2/users/me?context=edit").await?;
    let user_id = current_user["id"]
        .as_u64()
        .context("authenticated clone user omitted ID")?;
    ensure!(
        current_user["capabilities"]["edit_posts"] == true,
        "clone test user cannot create pages"
    );
    let runtime = ElementorService::new(&wp).runtime().await?;
    ensure!(
        runtime["elementor_core_version"] == "4.2.3",
        "unexpected Elementor Core runtime: {runtime}"
    );
    ensure!(
        runtime["elementor_pro_version"] == "4.2.2",
        "unexpected Elementor Pro runtime: {runtime}"
    );

    let bridge_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/mcp-bridge.php");
    let bridge_hash = Command::new("shasum")
        .args(["-a", "256"])
        .arg(&bridge_path)
        .output()
        .context("hash the bridge source used by the test")?;
    ensure!(
        bridge_hash.status.success(),
        "could not hash the bridge source used by the test"
    );
    let bridge_sha256 = String::from_utf8(bridge_hash.stdout)?
        .split_whitespace()
        .next()
        .context("shasum omitted the bridge source digest")?
        .to_owned();
    let before = php_result(STATE_SCRIPT, "atomic-css-before", &[])?;
    ensure!(
        before["bridge"] == bridge_sha256,
        "active bridge source differs from the verified runtime source"
    );
    ensure!(
        before["protected"]["8416"]["post"].is_string()
            && before["protected"]["22393"]["post"].is_string(),
        "protected page baseline is incomplete on the disposable child"
    );
    let suffix = unique_suffix();
    let class_name = format!("atomic-color-proof-{suffix}");
    let node_id = format!("{:07x}", std::process::id() as u32 & 0x0fffffff);
    let fixture = php_result(
        NATIVE_BOOTSTRAP,
        "atomic-css-fixture",
        &[
            ("MCP_TEST_USER_ID", user_id.to_string()),
            ("MCP_FIXTURE_SUFFIX", suffix.clone()),
            ("MCP_NODE_ID", node_id.clone()),
        ],
    )?;
    let page_id = fixture["page_id"]
        .as_u64()
        .context("native fixture omitted page ID")?;
    ensure!(
        fixture["class"] == class_name,
        "native fixture class did not match its unique selector"
    );
    let operation_id = format!("atomic_css_{}", unique_suffix());
    let mut needs_restore = false;
    let mut chrome: Option<ChromeProcess> = None;

    let proof: anyhow::Result<()> = async {
        let edit_page = wp.get(&format!("wp/v2/pages/{page_id}?context=edit")).await?;
        ensure!(edit_page["id"] == page_id, "native fixture is not editable through WordPress REST");
        let saved = get_page_document(&wp, page_id).await?;
        let initial: Value = serde_json::from_str(&saved.raw)?;
        ensure!(initial == fixture["document"], "native document readback differs from Elementor's canonical save payload");
        let page_url = edit_page["link"].as_str().context("published fixture omitted its guest URL")?;
        chrome = Some(ChromeProcess::start()?);
        let initial_browser = browser_style("initial", page_url, &class_name)?;
        ensure!(initial_browser["found"] == true, "guest HTML omitted the native class: {initial_browser}");
        ensure!(initial_browser["color"] == "rgb(255, 0, 0)", "native map insertion order did not determine the CSS cascade: {initial_browser}");
        ensure!(initial_browser["background"] == "rgb(17, 34, 51)", "guest baseline color is wrong: {initial_browser}");
        ensure!(initial_browser["minHeight"] == "50px", "guest baseline height is wrong: {initial_browser}");
        ensure!(initial_browser["width"] == "320px", "guest desktop style geometry is wrong: {initial_browser}");
        let initial_mobile = browser_style("mobile", page_url, &class_name)?;
        ensure!(initial_mobile["width"] == "280px", "guest mobile style geometry is wrong: {initial_mobile}");
        let initial_hover = browser_style("hover", page_url, &class_name)?;
        ensure!(initial_hover["opacity"] == "0.8", "guest hover style is wrong: {initial_hover}");
        let cache_before = php_result(STATE_SCRIPT, "atomic-css-page-before", &[("MCP_PAGE_ID", page_id.to_string())])?;
        ensure!(cache_before["fixture"]["document"] == initial, "native fixture baseline document differs from Elementor's canonical save payload");
        let artifact_scenarios = php_result(ATOMIC_ARTIFACT_SCENARIOS, "atomic-css-artifact-scenarios", &[("MCP_PAGE_ID", page_id.to_string()), ("MCP_TEST_USER_ID", user_id.to_string())])?;
        ensure!(artifact_scenarios["empty_styles_regenerated"] == true, "empty Atomic CSS fixture did not regenerate: {artifact_scenarios}");
        ensure!(artifact_scenarios["missing_leaf_status"] == "stale", "missing Atomic validity leaf was not rejected: {artifact_scenarios}");
        let after_artifact_scenarios = php_result(STATE_SCRIPT, "atomic-css-page-after-artifact-scenarios", &[("MCP_PAGE_ID", page_id.to_string())])?;
        ensure!(after_artifact_scenarios["fixture"] == cache_before["fixture"], "Atomic artifact scenarios did not restore the owned fixture state");

        let inventory = elementor_planning::parse_css_for_migration(None, Some(page_url)).await?;
        let (css_rule_index, css_rule) = inventory
            .rules
            .iter()
            .enumerate()
            .find(|(_, rule)| {
                rule.selector.contains(&class_name)
                    && rule.declarations.contains_key("background-color")
            })
            .context("native guest CSSOM omitted the local Atomic background rule")?;
        let matched_styles = elementor_planning::css_matched_styles(page_url, &css_rule.selector).await?;
        let converted = wp
            .post(
                "elementor/v1/css-to-atomic",
                &json!({"blocks":{"desktop":{"background-color":"#223344"}}}),
            )
            .await?;
        let props = converted
            .pointer("/data/desktop/props")
            .filter(|props| props.is_object() && !props.as_object().unwrap().is_empty())
            .context("native Atomic converter omitted CSS props")?
            .clone();
        let atomic_conversion = inventory
            .rules
            .iter()
            .enumerate()
            .map(|(index, _)| {
                (
                    index,
                    AtomicCssConversion {
                        settings: if index == css_rule_index {
                            props.clone()
                        } else {
                            json!({})
                        },
                        custom_css: None,
                        rejected: vec![],
                        capability: "elementor-native-css-converter".into(),
                    },
                )
            })
            .collect();
        let planned = elementor_planning::plan_css_migration(CssMigrationInput {
            context: PlanContext {
                site_url: wp.base_url().to_owned(),
                page_id,
                expected_revision: saved.revision.clone(),
                operation_id: operation_id.clone(),
            },
            document_tree: initial.clone().as_array().context("native fixture document omitted element array")?.clone(),
            css_text: None,
            source_url: Some(page_url.to_owned()),
            selector: Some(css_rule.selector.clone()),
            target_element_id: Some(node_id.clone()),
            matched_styles: Some(matched_styles),
            target: "atomic".into(),
            widget_type: None,
            classic_allowed_keys: None,
            atomic_conversion: Some(atomic_conversion),
            page_settings: None,
            custom_css_capability: false,
            breakpoints: Default::default(),
        })
        .await?;
        let planned_tree = planned
            .plan
            .document_patch
            .as_ref()
            .with_context(|| format!("native CSS migration planner produced no document patch; selector={:?}; inventory={:?}; source={}", planned.inventory.iter().map(|rule| &rule.selector).collect::<Vec<_>>(), planned.inventory, planned.plan.source.as_ref().unwrap_or(&Value::Null)))?;
        let style_class = format!("mcp-css-{node_id}");
        ensure!(planned_tree[0]["settings"]["classes"]["value"].as_array().is_some_and(|classes| classes.iter().any(|class| class == &style_class)), "planner did not bind the native style class: {planned_tree}");
        ensure!(planned_tree[0]["styles"][&style_class]["id"] == style_class, "planner did not serialize the matching native style ID: {planned_tree}");
        let original_style = initial[0]["styles"]
            .as_object()
            .and_then(|styles| styles.get(&class_name))
            .or_else(|| initial[0]["styles"].as_array().and_then(|styles| styles.iter().find(|style| style["id"] == class_name)))
            .context("native fixture readback omitted its original style definition")?;
        ensure!(planned_tree[0]["styles"][&class_name]["id"] == original_style["id"] && planned_tree[0]["styles"][&class_name]["variants"] == original_style["variants"], "planner did not preserve the existing native style map and its responsive/hover variants: {planned_tree}");
        ensure!(planned_tree[0]["styles"].as_object().is_some_and(|styles| styles.keys().map(String::as_str).collect::<Vec<_>>() == [class_name.as_str(), "z-native-order", "a-native-order", style_class.as_str()]), "planner changed the native style definition order or appended its class incorrectly: {planned_tree}");
        let pre_apply_cache = php_result(STATE_SCRIPT, "atomic-css-page-pre-apply", &[("MCP_PAGE_ID", page_id.to_string())])?;
        let applied = ApplyElementorPlan
            .run(
                json!({"plan":planned.plan}),
                &wp,
            )
            .await?;
        if applied.is_error == Some(true) {
            let after_error = get_page_document(&wp, page_id).await?;
            needs_restore = serde_json::from_str::<Value>(&after_error.raw)? != initial;
            anyhow::bail!("CAS color apply failed: {}", serde_json::to_string(&applied)?);
        }
        let receipt = tool_json(&applied)?;
        ensure!(receipt["applied"] == true, "CAS apply was not confirmed: {receipt}");
        needs_restore = true;
        let after_apply = get_page_document(&wp, page_id).await?;
        let applied_tree: Value = serde_json::from_str(&after_apply.raw)?;
        ensure!(applied_tree[0]["settings"]["classes"]["value"].as_array().is_some_and(|classes| classes.iter().any(|class| class == &style_class)), "CAS apply readback lost the bound native style class");
        let applied_style_ids = applied_tree[0]["styles"]
            .as_array()
            .map(|styles| styles.iter().filter_map(|style| style["id"].as_str()).collect::<Vec<_>>())
            .or_else(|| applied_tree[0]["styles"].as_object().map(|styles| styles.keys().map(String::as_str).collect::<Vec<_>>()));
        ensure!(applied_style_ids.as_deref() == Some([class_name.as_str(), "z-native-order", "a-native-order", style_class.as_str()].as_slice()), "native read-save cycle changed style map insertion order: {applied_tree}");
        let applied_cache = php_result(
            STATE_SCRIPT,
            "atomic-css-page-applied",
            &[
                ("MCP_PAGE_ID", page_id.to_string()),
                ("MCP_STYLE_CLASS", style_class.clone()),
                ("MCP_OPERATION_ID", operation_id.clone()),
            ],
        )?;
        ensure!(applied_cache["operation_snapshot"]["has_before"] == true && applied_cache["operation_snapshot"]["has_after"] == true && applied_cache["operation_snapshot"]["has_after_fingerprint"] == true, "generic operation journal omitted its paired full-state snapshot: {}", applied_cache["operation_snapshot"]);

        let reloaded = browser_style("reload", page_url, &class_name)?;
        let fresh_guest = browser_style("new", page_url, &class_name)?;
        let applied_mobile = browser_style("mobile", page_url, &class_name)?;
        let applied_hover = browser_style("hover", page_url, &class_name)?;
        ensure!(reloaded["color"] == "rgb(255, 0, 0)", "native read/save changed the conflicting-class computed color: {reloaded}");
        let applied_guest_matches = reloaded["classes"].as_array().is_some_and(|classes| classes.iter().any(|class| class == &style_class))
            && reloaded["background"] == "rgb(34, 51, 68)"
            && reloaded["minHeight"] == "50px"
            && reloaded["width"] == "320px"
            && fresh_guest["classes"].as_array().is_some_and(|classes| classes.iter().any(|class| class == &style_class))
            && fresh_guest["background"] == "rgb(34, 51, 68)"
            && applied_mobile["width"] == "280px"
            && applied_hover["opacity"] == "0.8";

        php_result(FOREIGN_CONFLICT_SCRIPT, "atomic-css-foreign-meta", &[("MCP_PAGE_ID", page_id.to_string()), ("MCP_ACTION", "add_meta".into())])?;
        let unchanged_revision = get_page_document(&wp, page_id).await?.revision;
        let foreign_meta_restore = RestoreElementorSnapshot.run(json!({"page_id":page_id,"operation_id":operation_id,"expected_revision":unchanged_revision}), &wp).await;
        let foreign_meta_restore_text = match foreign_meta_restore {
            Ok(response) => serde_json::to_string(&response)?,
            Err(error) => format!("{error:#}"),
        };
        ensure!(foreign_meta_restore_text.contains("WP API 409 Conflict") && foreign_meta_restore_text.contains("resource_restore_conflict") && !foreign_meta_restore_text.contains("\"restored\":true"), "full restore did not reject foreign metadata with an explicit conflict: {foreign_meta_restore_text}");
        let foreign_meta_state = php_result(FOREIGN_CONFLICT_SCRIPT, "atomic-css-foreign-meta-state", &[("MCP_PAGE_ID", page_id.to_string()), ("MCP_ACTION", "inspect".into())])?;
        ensure!(foreign_meta_state["foreign_meta"] == "foreign", "foreign metadata was overwritten after rejected restore");
        php_result(FOREIGN_CONFLICT_SCRIPT, "atomic-css-foreign-meta-clean", &[("MCP_PAGE_ID", page_id.to_string()), ("MCP_ACTION", "delete_meta".into())])?;

        php_result(FOREIGN_CONFLICT_SCRIPT, "atomic-css-foreign-title", &[("MCP_PAGE_ID", page_id.to_string()), ("MCP_ACTION", "foreign_title".into())])?;
        let title_restore = RestoreElementorSnapshot.run(json!({"page_id":page_id,"operation_id":operation_id,"expected_revision":unchanged_revision}), &wp).await;
        let title_restore_text = match title_restore {
            Ok(response) => serde_json::to_string(&response)?,
            Err(error) => format!("{error:#}"),
        };
        ensure!(title_restore_text.contains("WP API 409 Conflict") && title_restore_text.contains("resource_restore_conflict") && !title_restore_text.contains("\"restored\":true"), "full restore did not reject a foreign title with an explicit conflict: {title_restore_text}");
        let foreign_title_state = php_result(FOREIGN_CONFLICT_SCRIPT, "atomic-css-foreign-title-state", &[("MCP_PAGE_ID", page_id.to_string()), ("MCP_ACTION", "inspect".into())])?;
        ensure!(foreign_title_state["title"].as_str().is_some_and(|title| title.starts_with("Foreign title ")), "foreign title was overwritten after rejected restore");
        php_result(FOREIGN_CONFLICT_SCRIPT, "atomic-css-foreign-title-clean", &[("MCP_PAGE_ID", page_id.to_string()), ("MCP_ACTION", "restore_title".into()), ("MCP_ORIGINAL_TITLE", format!("Atomic color proof {suffix}"))])?;

        restore(&wp, page_id, &operation_id, true).await?;
        needs_restore = false;
        let restored = get_page_document(&wp, page_id).await?;
        let restored_tree: Value = serde_json::from_str(&restored.raw)?;
        ensure!(restored_tree == initial, "generic restore did not recover the original native document");
        let restored_cache = php_result(STATE_SCRIPT, "atomic-css-page-restored", &[("MCP_PAGE_ID", page_id.to_string())])?;
        let restored_guest = browser_style("reload", page_url, &class_name)?;
        ensure!(restored_cache["fixture"] == cache_before["fixture"], "generic restore did not recover the page Atomic CSS cache/artifact state; before={}, pre_apply={}, operation_snapshot={}, after={}; applied_guest={}; restored_guest={}; generated_css={}", cache_before["fixture"], pre_apply_cache["fixture"], applied_cache["operation_snapshot"], restored_cache["fixture"], json!({"reload":reloaded,"fresh":fresh_guest}), restored_guest, applied_cache["fixture"]["style_rules"]);
        ensure!(applied_guest_matches, "normal-cache reload and new guest target did not render the planner-bound Atomic style; reload={reloaded}; fresh={fresh_guest}; restored_guest={restored_guest}; generated_css={}", applied_cache["fixture"]["style_rules"]);
        ensure!(restored_guest["background"] == "rgb(17, 34, 51)", "guest page did not return to the original Atomic color after restore: {restored_guest}");
        ensure!(restored_guest["minHeight"] == "50px", "restore changed the original Atomic min-height: {restored_guest}");
        Ok(())
    }
    .await;

    let recovery = if needs_restore {
        restore(&wp, page_id, &operation_id, true).await
    } else {
        Ok(())
    };
    let operation_ids = json!([operation_id]);
    let proof_error = proof.as_ref().err().map(|error| format!("{error:#}"));
    let cleanup = php_result(
        CLEANUP_SCRIPT,
        "atomic-css-cleanup",
        &[
            ("MCP_PAGE_ID", page_id.to_string()),
            ("MCP_OPERATION_IDS", operation_ids.to_string()),
        ],
    );
    drop(chrome.take());
    recovery.map_err(|error| {
        anyhow::anyhow!(
            "proof failed: {}; exact restore failed: {error:#}",
            proof_error.as_deref().unwrap_or("none")
        )
    })?;
    cleanup.context("remove owned Atomic CSS fixture, journals, and artifacts")?;
    let after = php_result(STATE_SCRIPT, "atomic-css-after", &[])?;
    ensure!(
        after["bridge"] == bridge_sha256,
        "active bridge source changed during the proof"
    );
    ensure!(
        after["protected"] == before["protected"],
        "protected page 8416 or 22393 post/meta state changed"
    );
    ensure!(
        after["kit"] == before["kit"],
        "active kit or Global Classes collection changed during the proof"
    );
    ensure!(
        after["atomic_cache"] == before["atomic_cache"],
        "Atomic cache validity state differs after exact fixture cleanup"
    );
    ensure!(
        after["storedcss_print_method"] == before["storedcss_print_method"],
        "stored CSS print method changed during the proof"
    );
    ensure!(
        after["artifacts"] == before["artifacts"],
        "protected or global Atomic CSS artifact files changed"
    );
    proof
}
