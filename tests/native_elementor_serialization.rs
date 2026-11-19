use anyhow::{Context, ensure};
use mcp_for_page_builders::tools::elementor_planning::{
    AtomicRuntimeSchema, ConvertElementsInput, PlanContext, plan_convert_elements,
};
use mcp_for_page_builders::{
    elementor::{ElementorService, get_page_document},
    tools::{
        Tool, elementor_plan_tools::ApplyElementorPlan, elementor_planning_tools::ConvertElements,
    },
    wp::WpClient,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    process::Command,
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

fn docker(args: &[&str]) -> std::process::Output {
    let output = Command::new("docker")
        .args(args)
        .output()
        .expect("docker must be available for the native Elementor check");
    assert!(
        output.status.success(),
        "docker command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn save_dynamic_fixture(page_id: u64, user_id: u64) -> anyhow::Result<Value> {
    let nonce = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    );
    let php_local = std::env::temp_dir().join(format!("mcp-native-document-{nonce}.php"));
    let result_local = std::env::temp_dir().join(format!("mcp-native-document-{nonce}.json"));
    let result_remote = format!("/tmp/mcp-native-document-{nonce}.json");
    let php_remote = format!("/tmp/mcp-native-document-{nonce}.php");
    fs::write(
        &php_local,
        r#"<?php
$uid=(int)getenv('MCP_TEST_USER_ID');
$id=(int)getenv('MCP_PAGE_ID');
wp_set_current_user($uid);
if (!current_user_can('edit_post',$id)) throw new RuntimeException('test user cannot edit native fixture');
$dynamic=Elementor\Plugin::instance()->dynamic_tags->tag_data_to_tag_text(null,'post-title',[]);
if (!is_string($dynamic)||$dynamic==='') throw new RuntimeException('registered post-title dynamic tag is unavailable');
$source=['id'=>'7200001','elType'=>'widget','widgetType'=>'heading','settings'=>['title'=>'Page title','header_size'=>'h2','__dynamic__'=>['title'=>$dynamic]],'elements'=>[]];
$instance=Elementor\Plugin::instance()->elements_manager->create_element_instance($source);
if (!$instance||!is_callable([$instance,'get_data_for_save'])) throw new RuntimeException('native heading serializer unavailable');
$canonical=$instance->get_data_for_save();
$document=Elementor\Plugin::instance()->documents->get($id);
if (!$document||!$document->save(['elements'=>[$canonical]])) throw new RuntimeException('native dynamic document save failed');
$stored=json_decode((string)get_post_meta($id,'_elementor_data',true),true,512,JSON_THROW_ON_ERROR);
if (!is_array($stored)||($stored[0]['settings']['title']??null)!=='Page title'||($stored[0]['settings']['__dynamic__']['title']??null)!==$dynamic) throw new RuntimeException('native serializer did not preserve the registered dynamic binding');
file_put_contents(getenv('MCP_RESULT_PATH'),wp_json_encode($stored));
"#,
    )?;
    let execution = (|| {
        let output = Command::new("docker")
            .args([
                "cp",
                php_local.to_str().context("temporary PHP path is UTF-8")?,
                &format!("{}:{php_remote}", config().container),
            ])
            .output()?;
        ensure!(
            output.status.success(),
            "docker cp failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let output = Command::new("docker")
            .args([
                "exec",
                "-e",
                &format!("MCP_TEST_USER_ID={user_id}"),
                "-e",
                &format!("MCP_PAGE_ID={page_id}"),
                "-e",
                &format!("MCP_RESULT_PATH={result_remote}"),
                "-u",
                config().docker_user.as_str(),
                config().container.as_str(),
                "wp",
                config().wp_path_arg.as_str(),
                "eval-file",
                &php_remote,
            ])
            .output()?;
        ensure!(
            output.status.success(),
            "native dynamic fixture save failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let output = Command::new("docker")
            .args([
                "cp",
                &format!("{}:{result_remote}", config().container),
                result_local
                    .to_str()
                    .context("temporary result path is UTF-8")?,
            ])
            .output()?;
        ensure!(
            output.status.success(),
            "docker cp of native dynamic readback failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(serde_json::from_slice(&fs::read(&result_local)?)?)
    })();
    let cleanup = Command::new("docker")
        .args([
            "exec",
            config().container.as_str(),
            "rm",
            "-f",
            &result_remote,
            &php_remote,
        ])
        .output();
    let _ = fs::remove_file(result_local);
    let _ = fs::remove_file(php_local);
    let result = execution?;
    let cleanup = cleanup?;
    ensure!(
        cleanup.status.success(),
        "could not remove owned native fixture helper files: {}",
        String::from_utf8_lossy(&cleanup.stderr)
    );
    Ok(result)
}

fn save_selected_fixture(page_id: u64, user_id: u64) -> anyhow::Result<Value> {
    let php = r#"$uid=__USER_ID__; $id=__PAGE_ID__; wp_set_current_user($uid);
if (!current_user_can('edit_post', $id)) throw new RuntimeException('test user cannot edit selected-conversion fixture');
$dynamic=Elementor\Plugin::instance()->dynamic_tags->tag_data_to_tag_text(null, 'post-title', []);
if (!is_string($dynamic)||$dynamic==='') throw new RuntimeException('registered post-title dynamic tag is unavailable');
$source=[
 ['id'=>'8100001','elType'=>'section','settings'=>['flex_gap'=>['size'=>16,'unit'=>'px']],'elements'=>[
  ['id'=>'8100002','elType'=>'column','settings'=>['_column_size'=>100,'_column_size_tablet'=>75],'elements'=>[
   ['id'=>'8100003','elType'=>'widget','widgetType'=>'heading','settings'=>['title'=>'Selected heading','header_size'=>'h2'],'elements'=>[]],
   ['id'=>'8100004','elType'=>'widget','widgetType'=>'button','settings'=>['text'=>'Apply now','link'=>['url'=>'https://example.test/apply','is_external'=>true]],'elements'=>[]],
   ['id'=>'8100005','elType'=>'widget','widgetType'=>'image','settings'=>['image'=>['id'=>0,'url'=>'https://example.test/selected.jpg','alt'=>'Selected media'],'image_size'=>'full','link_to'=>'custom','link'=>['url'=>'https://example.test/media/','is_external'=>true]],'elements'=>[]],
  ]],
 ]],
 ['id'=>'8100006','elType'=>'widget','widgetType'=>'heading','settings'=>['title'=>'Unselected heading','header_size'=>'h3'],'elements'=>[]],
 ['id'=>'8100007','elType'=>'widget','widgetType'=>'heading','settings'=>['title'=>'Bound dynamic title','header_size'=>'h2','__dynamic__'=>['title'=>$dynamic]],'elements'=>[]],
 ['id'=>'8100008','elType'=>'container','settings'=>['css_classes'=>'wrapper-class'],'elements'=>[
  ['id'=>'8100009','elType'=>'widget','widgetType'=>'heading','settings'=>['title'=>'Wrapper child'],'elements'=>[]]
 ]],
 ['id'=>'8100010','elType'=>'widget','widgetType'=>'button','settings'=>['text'=>'Keep nofollow','link'=>['url'=>'https://example.test/nofollow','is_external'=>true,'nofollow'=>'yes']],'elements'=>[]],
];
$manager=Elementor\Plugin::instance()->elements_manager;
$elements=[];
foreach ($source as $node) { $instance=$manager->create_element_instance($node); if (!$instance||!is_callable([$instance,'get_data_for_save'])) throw new RuntimeException('native Classic fixture element serializer unavailable'); $elements[]=$instance->get_data_for_save(); }
$document=Elementor\Plugin::instance()->documents->get($id);
if (!$document||!$document->save(['elements'=>$elements])) throw new RuntimeException('native selected-conversion source save failed');
$stored=json_decode((string)get_post_meta($id,'_elementor_data',true),true,512,JSON_THROW_ON_ERROR);
if (!is_array($stored)||count($stored)!==5) throw new RuntimeException('native selected-conversion source readback is incomplete');
echo 'MCP_SELECTED_SOURCE:' . wp_json_encode($stored);
"#
    .replace("__USER_ID__", &user_id.to_string())
    .replace("__PAGE_ID__", &page_id.to_string());
    let output = Command::new("docker")
        .args([
            "exec",
            "-u",
            config().docker_user.as_str(),
            config().container.as_str(),
            "wp",
            config().wp_path_arg.as_str(),
            "eval",
            &php,
        ])
        .output()
        .context("save selected-conversion fixture through native Elementor")?;
    ensure!(
        output.status.success(),
        "native selected-conversion source save failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout)?;
    let (_, source) = stdout
        .split_once("MCP_SELECTED_SOURCE:")
        .context("native selected-conversion save omitted its readback marker")?;
    Ok(serde_json::from_str(source.trim())?)
}

fn native_resave_document(page_id: u64, user_id: u64, tree: &Value) -> anyhow::Result<Value> {
    use base64::Engine;

    let encoded = base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(tree)?);
    let php = format!(
        "$uid = {user_id}; $id = {page_id}; wp_set_current_user($uid); if (!current_user_can('edit_post', $id)) throw new RuntimeException('test user cannot edit native roundtrip fixture'); $raw = base64_decode('{encoded}', true); if (!is_string($raw)) throw new RuntimeException('native roundtrip payload could not be decoded'); $source = json_decode($raw, true, 512, JSON_THROW_ON_ERROR); if (!is_array($source)) throw new RuntimeException('native roundtrip document is not an array'); $manager = Elementor\\Plugin::instance()->elements_manager; $elements = []; foreach ($source as $node) {{ $instance = $manager->create_element_instance($node); if (!$instance || !is_callable([$instance, 'get_data_for_save'])) throw new RuntimeException('native Elementor serializer rejected an element'); $elements[] = $instance->get_data_for_save(); }} $document = Elementor\\Plugin::instance()->documents->get($id); if (!$document || !$document->save(['elements' => $elements])) throw new RuntimeException('native Elementor Document::save failed'); $stored = json_decode((string) get_post_meta($id, '_elementor_data', true), true, 512, JSON_THROW_ON_ERROR); if (!is_array($stored)) throw new RuntimeException('native Elementor readback is not a document array'); echo 'MCP_NATIVE_DOCUMENT_SAVE:' . wp_json_encode($stored);"
    );
    let output = Command::new("docker")
        .args([
            "exec",
            "-u",
            config().docker_user.as_str(),
            config().container.as_str(),
            "wp",
            config().wp_path_arg.as_str(),
            "eval",
            &php,
        ])
        .output()
        .context("roundtrip selected conversion through Elementor Document::save")?;
    ensure!(
        output.status.success(),
        "native Elementor Document::save failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout)?;
    let (_, stored) = stdout
        .split_once("MCP_NATIVE_DOCUMENT_SAVE:")
        .context("native Elementor Document::save omitted its readback marker")?;
    Ok(serde_json::from_str(stored.trim())?)
}

fn native_revision_ids(page_id: u64) -> anyhow::Result<Vec<u64>> {
    let query = format!(
        "SELECT ID FROM lakd_posts WHERE post_type = 'revision' AND post_parent = {page_id} ORDER BY ID"
    );
    let output = docker(&[
        "exec",
        "-u",
        config().docker_user.as_str(),
        config().container.as_str(),
        "wp",
        config().wp_path_arg.as_str(),
        "db",
        "query",
        "--batch",
        "--raw",
        "--skip-column-names",
        &query,
    ]);
    String::from_utf8(output.stdout)?
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.parse::<u64>().context("native revision ID is numeric"))
        .collect()
}

fn save_native_roundtrip_evidence(
    page_id: u64,
    phase: &str,
    before: &Value,
    after: &Value,
) -> anyhow::Result<String> {
    let path = format!("/tmp/karjera-mcp-integration/native-document-{page_id}-{phase}.json");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .with_context(|| format!("create private native roundtrip evidence {path}"))?;
    serde_json::to_writer_pretty(&mut file, &json!({"before": before, "after": after}))?;
    file.write_all(b"\n")?;
    Ok(path)
}

fn native_rendered_html(page_id: u64) -> anyhow::Result<String> {
    let php = format!(
        "$html = Elementor\\Plugin::instance()->frontend->get_builder_content_for_display({page_id}, false); echo 'MCP_NATIVE_RENDER:' . $html;"
    );
    let output = Command::new("docker")
        .args([
            "exec",
            "-u",
            config().docker_user.as_str(),
            config().container.as_str(),
            "wp",
            config().wp_path_arg.as_str(),
            "eval",
            &php,
        ])
        .output()
        .context("render native Elementor conversion fixture")?;
    ensure!(
        output.status.success(),
        "native Elementor render failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout)?;
    Ok(stdout
        .split_once("MCP_NATIVE_RENDER:")
        .context("native Elementor renderer omitted its output marker")?
        .1
        .to_owned())
}

#[test]
fn converted_widgets_and_layout_validate_and_roundtrip_in_active_elementor() {
    let nonce = std::process::id();
    let local_schema = std::env::temp_dir().join(format!("elementor-schema-{nonce}.json"));
    let local_elements = std::env::temp_dir().join(format!("elementor-elements-{nonce}.json"));
    let local_schema_php = std::env::temp_dir().join(format!("elementor-schema-{nonce}.php"));
    let local_validate_php = std::env::temp_dir().join(format!("elementor-validate-{nonce}.php"));
    let remote_schema = format!("/tmp/elementor-schema-{nonce}.json");
    let remote_elements = format!("/tmp/elementor-elements-{nonce}.json");
    let remote_schema_php = format!("/tmp/elementor-schema-{nonce}.php");
    let remote_validate_php = format!("/tmp/elementor-validate-{nonce}.php");

    fs::write(&local_schema_php, r#"<?php
$manager = Elementor\Plugin::instance()->elements_manager->get_element_types();
$out = ['elements' => [], 'atomic_style_schema' => Elementor\Modules\AtomicWidgets\Styles\Style_Schema::get()];
foreach (['e-heading','e-paragraph','e-image','e-button','e-flexbox'] as $name) {
    $type = $manager[$name] ?? Elementor\Plugin::instance()->widgets_manager->get_widget_types()[$name] ?? null;
    if (!$type || !is_callable([$type, 'get_props_schema'])) throw new RuntimeException("Missing native schema for $name");
    $initial = is_callable([$type, 'get_initial_config']) ? $type->get_initial_config() : [];
    $out['elements'][$name] = ['props_schema' => $type->get_props_schema(), 'initial_config' => $initial];
}

file_put_contents('"#.to_owned() + &remote_schema + r#"', wp_json_encode($out));
"#).expect("write schema probe");
    docker(&[
        "cp",
        local_schema_php.to_str().unwrap(),
        &format!("{}:{remote_schema_php}", config().container),
    ]);
    docker(&[
        "exec",
        "-u",
        config().docker_user.as_str(),
        config().container.as_str(),
        "wp",
        config().wp_path_arg.as_str(),
        "eval-file",
        &remote_schema_php,
    ]);
    docker(&[
        "cp",
        &format!("{}:{remote_schema}", config().container),
        local_schema.to_str().unwrap(),
    ]);

    let native: Value =
        serde_json::from_slice(&fs::read(&local_schema).expect("native schemas were written"))
            .expect("native schemas are JSON");
    let mut runtime_elements = serde_json::Map::new();
    for (name, element_schema) in native["elements"].as_object().expect("element schemas") {
        runtime_elements.insert(name.clone(), element_schema["initial_config"].clone());
    }
    let mut schemas =
        AtomicRuntimeSchema::from_runtime(&json!({"elements":runtime_elements}), &native);
    for (name, element_schema) in native["elements"].as_object().unwrap() {
        schemas.add_element_schema(name, element_schema);
    }

    let tree = vec![json!({
        "id":"10000001","elType":"section","settings":{"flex_gap":{"size":12,"unit":"px"}},"elements":[{
            "id":"10000002","elType":"column","settings":{"_column_size":100,"_column_size_tablet":75},"elements":[
                {"id":"10000003","elType":"widget","widgetType":"heading","settings":{"title":"Heading &amp; title","header_size":"h2"},"elements":[]},
                {"id":"10000004","elType":"widget","widgetType":"text-editor","settings":{"editor":"<p>Paragraph &amp; text</p>"},"elements":[]},
                {"id":"10000005","elType":"widget","widgetType":"image","settings":{"image":{"url":"https://example.com/image.jpg","alt":"Photo"},"image_size":"full"},"elements":[]},
                {"id":"10000006","elType":"widget","widgetType":"button","settings":{"text":"Open","link":{"url":"https://example.com/","is_external":true}},"elements":[]}
            ]
        }]
    })];
    let context = PlanContext {
        site_url: "https://example.test".into(),
        page_id: 1,
        expected_revision: "fixture-revision".into(),
        operation_id: "native-check-1".into(),
    };
    let plan = plan_convert_elements(ConvertElementsInput {
        context: context.clone(),
        direction: "classic_to_atomic".into(),
        elements: tree,
        schemas: Some(schemas.clone()),
    })
    .unwrap();
    assert!(
        plan.retained.is_empty(),
        "conversion retained unexpected subtrees: {:?}",
        plan.retained
    );
    let atomic = plan
        .document_patch
        .expect("complete conversion is executable");
    assert!(
        atomic.is_array(),
        "document_patch is the full canonical tree"
    );
    assert_eq!(
        atomic[0]["elements"][0]["styles"]["mcp-layout-10000002-tablet"]["variants"][0]["meta"]["breakpoint"],
        "tablet"
    );
    assert!(
        atomic[0]["elements"][0]["elements"][2]["settings"]["image"]["value"]["src"]["value"]
            .get("id")
            .is_none()
    );
    assert_eq!(
        atomic[0]["elements"][0]["elements"][1]["settings"]["paragraph"]["value"]["content"]["value"],
        "Paragraph &amp; text"
    );

    fs::write(&local_elements, serde_json::to_vec(&atomic).unwrap()).expect("write converted tree");
    let validator = format!(
        r#"<?php
$tree = json_decode(file_get_contents('{remote_elements}'), true, 512, JSON_THROW_ON_ERROR);
$widgets = Elementor\Plugin::instance()->widgets_manager->get_widget_types();
$elements = Elementor\Plugin::instance()->elements_manager->get_element_types();
$style_parser = Elementor\Modules\AtomicWidgets\Parsers\Style_Parser::make(Elementor\Modules\AtomicWidgets\Styles\Style_Schema::get());
$walk = function($nodes) use (&$walk, $widgets, $elements, $style_parser) {{
  foreach ($nodes as $node) {{
    $type = ($node['elType'] ?? '') === 'widget' ? ($widgets[$node['widgetType'] ?? ''] ?? null) : ($elements[$node['elType'] ?? ''] ?? null);
    if (!$type) throw new RuntimeException('Unregistered converted element: ' . ($node['widgetType'] ?? $node['elType'] ?? 'unknown'));
    if (is_callable([$type, 'get_props_schema'])) foreach (($node['settings'] ?? []) as $key => $value) {{
      $schema = call_user_func([get_class($type), 'get_props_schema']);
      if (!isset($schema[$key]) || !$schema[$key]->validate($value)) throw new RuntimeException('Native prop validator rejected ' . ($node['widgetType'] ?? $node['elType']) . '.' . $key);
    }}
    foreach (($node['styles'] ?? []) as $style) {{ $parsed = $style_parser->parse($style); if (!$parsed->is_valid()) throw new RuntimeException('Native style parser rejected ' . ($node['id'] ?? 'unknown') . ': ' . $parsed->errors()->to_string()); }}
    $walk($node['elements'] ?? []);
  }}
}};
$walk($tree);
$exported = apply_filters('elementor/template_library/sources/local/export/elements', $tree);
$imported = apply_filters('elementor/template_library/sources/local/import/elements', $exported);
$root = $imported[0] ?? null;
if (!is_array($root) || !is_array($root['styles'] ?? null) || !$root['styles']) throw new RuntimeException('Native Atomic template export/import lost the root style map.');
$old_ids = array_keys($root['styles']);
foreach ($root['styles'] as $key => $style) {{
  if (($style['id'] ?? null) !== $key) throw new RuntimeException('Native export/import changed a style map key without matching its definition ID.');
  if (!in_array($key, $root['settings']['classes']['value'] ?? [], true)) throw new RuntimeException('Native export/import lost the typed style class binding.');
}}
$root['id'] = '61000001';
$rebound = apply_filters('elementor/document/element/replace_id', $root);
if (!is_array($rebound['styles'] ?? null) || array_intersect($old_ids, array_keys($rebound['styles']))) throw new RuntimeException('Native Styles_Ids_Modifier did not remap the style map keys.');
foreach ($rebound['styles'] as $key => $style) {{
  if (($style['id'] ?? null) !== $key) throw new RuntimeException('Native Styles_Ids_Modifier emitted a mismatched style key and definition ID.');
  if (!in_array($key, $rebound['settings']['classes']['value'] ?? [], true)) throw new RuntimeException('Native Styles_Ids_Modifier did not rebind the typed classes prop.');
}}
file_put_contents('{remote_elements}.ok', 'valid');
file_put_contents('{remote_elements}.styles-ok', 'valid');
"#
    );
    fs::write(&local_validate_php, validator).expect("write native validator");
    docker(&[
        "cp",
        local_elements.to_str().unwrap(),
        &format!("{}:{remote_elements}", config().container),
    ]);
    docker(&[
        "cp",
        local_validate_php.to_str().unwrap(),
        &format!("{}:{remote_validate_php}", config().container),
    ]);
    docker(&[
        "exec",
        "-u",
        config().docker_user.as_str(),
        config().container.as_str(),
        "wp",
        config().wp_path_arg.as_str(),
        "eval-file",
        &remote_validate_php,
    ]);
    docker(&[
        "exec",
        config().container.as_str(),
        "test",
        "-f",
        &format!("{remote_elements}.ok"),
    ]);
    docker(&[
        "exec",
        config().container.as_str(),
        "test",
        "-f",
        &format!("{remote_elements}.styles-ok"),
    ]);

    let reverse = plan_convert_elements(ConvertElementsInput {
        context,
        direction: "atomic_to_classic".into(),
        elements: serde_json::from_value(atomic).unwrap(),
        schemas: Some(schemas),
    })
    .unwrap();
    assert!(
        reverse.retained.is_empty(),
        "roundtrip retained unexpected subtrees: {:?}",
        reverse.retained
    );
    let restored = reverse.document_patch.unwrap();
    assert_eq!(restored[0]["elType"], "section");
    assert_eq!(restored[0]["settings"]["flex_gap"]["size"], 12.0);
    assert_eq!(
        restored[0]["elements"][0]["settings"]["_column_size_tablet"],
        75.0
    );
    assert_eq!(
        restored[0]["elements"][0]["elements"][1]["settings"]["editor"],
        "<p>Paragraph &amp; text</p>"
    );

    for path in [
        local_schema,
        local_elements,
        local_schema_php,
        local_validate_php,
    ] {
        fs::remove_file(path).expect("remove native check temp file");
    }
    docker(&[
        "exec",
        config().container.as_str(),
        "rm",
        "-f",
        &remote_schema,
        &remote_elements,
        &remote_schema_php,
        &remote_validate_php,
        &format!("{remote_elements}.ok"),
        &format!("{remote_elements}.styles-ok"),
    ]);
}

#[tokio::test]
async fn converted_document_saves_reloads_and_converts_back_without_losing_supported_values() {
    let config = config();
    let wp = WpClient::new(&config.url, &config.user, &config.password);
    let current_user = wp
        .get("wp/v2/users/me?context=edit")
        .await
        .expect("read authenticated test user");
    let user_id = current_user["id"]
        .as_u64()
        .expect("authenticated test user ID");
    let marker = format!(
        "native_serialization_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock is after the Unix epoch")
            .as_nanos()
    );
    let title = format!("MCP {marker}");
    let source = json!([
        {"id":"7100001","elType":"section","settings":{"flex_gap":{"size":12,"unit":"px"},"css_classes":"mcp-native-section"},"elements":[
            {"id":"7100002","elType":"column","settings":{"_column_size":100,"_column_size_tablet":75,"css_classes":"mcp-native-column"},"elements":[
                {"id":"7100003","elType":"widget","widgetType":"heading","settings":{"title":"Roundtrip heading","header_size":"h2"},"elements":[]},
                {"id":"7100004","elType":"widget","widgetType":"text-editor","settings":{"editor":"<p>Roundtrip paragraph</p>"},"elements":[]},
                {"id":"7100005","elType":"widget","widgetType":"image","settings":{"image":{"id":0,"url":"https://example.test/roundtrip.jpg","alt":"Roundtrip image"},"image_size":"full"},"elements":[]},
                {"id":"7100006","elType":"widget","widgetType":"button","settings":{"text":"Apply","link":{"url":"https://example.test/apply","is_external":true}},"elements":[]}
            ]}
        ]}
    ]);
    let created = wp.post("wp/v2/pages", &json!({"title":title,"status":"draft","meta":{"_elementor_data":serde_json::to_string(&source).expect("encode source"),"_elementor_edit_mode":"builder"}})).await.expect("create isolated native roundtrip draft");
    let page_id = created["id"]
        .as_u64()
        .expect("WordPress returned draft page ID");
    eprintln!("native roundtrip test owns draft page {page_id}");
    let operation_id = format!("native_roundtrip_{marker}");
    let mut auxiliary_page_ids = Vec::new();
    let run: anyhow::Result<()> = async {
        let before = get_page_document(&wp, page_id).await.context("read source draft")?;
        let before_tree: Value = serde_json::from_str(&before.raw).context("source document is JSON")?;
        ensure!(before_tree == source, "WordPress did not persist the exact source tree");
        let active_flexbox_schema = ElementorService::new(&wp).element_schema("e-flexbox").await.context("read native Atomic flexbox schema")?;

        let planned = ConvertElements.run(json!({"page_id":page_id,"operation_id":operation_id,"direction":"classic_to_atomic"}), &wp).await.context("plan classic to Atomic conversion")?;
        ensure!(planned.is_error != Some(true), "conversion tool returned an error: {}", serde_json::to_string(&planned)?);
        let plan: Value = serde_json::from_str(serde_json::to_value(&planned)?["content"][0]["text"].as_str().context("conversion returned text")?).context("conversion plan is JSON")?;
        ensure!(plan["retained"].as_array().is_some_and(Vec::is_empty), "supported source tree was retained: {}; native flexbox class schema={}", plan["retained"], active_flexbox_schema.pointer("/props_schema/classes").unwrap_or(&Value::Null));
        ensure!(plan["document_patch"].is_array(), "conversion did not produce a full tree");
        let applied = ApplyElementorPlan.run(json!({"plan":plan}), &wp).await.context("apply compare-and-set conversion")?;
        ensure!(applied.is_error != Some(true), "Atomic save failed: {}", serde_json::to_string(&applied)?);
        let applied: Value = serde_json::from_str(serde_json::to_value(&applied)?["content"][0]["text"].as_str().context("apply returned text")?).context("apply response is JSON")?;
        ensure!(applied["applied"] == true, "Atomic save was not confirmed: {applied}");

        let atomic_doc = get_page_document(&wp, page_id).await.context("reload saved Atomic document")?;
        let atomic: Value = serde_json::from_str(&atomic_doc.raw).context("saved Atomic document is JSON")?;
        ensure!(atomic[0]["elements"][0]["elements"][0]["widgetType"] == "e-heading", "saved document did not contain the native heading");
        ensure!(atomic[0]["settings"]["classes"]["value"].as_array().is_some_and(|classes| classes.iter().any(|class| class == "mcp-native-section") && classes.iter().any(|class| class == "mcp-layout-7100001")), "native layout save lost the source and generated style class bindings");
        ensure!(atomic[0]["styles"]["mcp-layout-7100001"]["id"] == "mcp-layout-7100001" && atomic[0]["styles"]["mcp-layout-7100001"]["variants"][0]["meta"]["breakpoint"] == "desktop", "native layout save lost its bound desktop style definition");
        ensure!(atomic[0]["elements"][0]["settings"]["classes"]["value"].as_array().is_some_and(|classes| classes.iter().any(|class| class == "mcp-native-column") && classes.iter().any(|class| class == "mcp-layout-7100002-tablet")), "native responsive layout save lost the source class or tablet style binding");
        let reverse = ConvertElements.run(json!({"page_id":page_id,"operation_id":format!("{operation_id}_reverse"),"direction":"atomic_to_classic"}), &wp).await.context("plan Atomic to classic conversion from reloaded document")?;
        ensure!(reverse.is_error != Some(true), "reverse conversion tool returned an error: {}", serde_json::to_string(&reverse)?);
        let reverse_plan: Value = serde_json::from_str(serde_json::to_value(&reverse)?["content"][0]["text"].as_str().context("reverse conversion returned text")?).context("reverse plan is JSON")?;
        ensure!(reverse_plan["retained"].as_array().is_some_and(Vec::is_empty), "saved Atomic tree cannot convert back losslessly: retained={}, tree={atomic}", reverse_plan["retained"]);
        let applied = ApplyElementorPlan.run(json!({"plan":reverse_plan}), &wp).await.context("apply reverse compare-and-set conversion")?;
        ensure!(applied.is_error != Some(true), "classic save failed: {}", serde_json::to_string(&applied)?);
        let final_doc = get_page_document(&wp, page_id).await.context("reload restored classic document")?;
        let restored: Value = serde_json::from_str(&final_doc.raw).context("restored document is JSON")?;
        ensure!(restored[0]["settings"]["flex_gap"]["size"] == 12, "section spacing changed: {}", restored[0]);
        ensure!(restored[0]["elements"][0]["elements"][0]["settings"]["title"] == "Roundtrip heading", "heading text changed");
        ensure!(restored[0]["elements"][0]["elements"][1]["settings"]["editor"] == "<p>Roundtrip paragraph</p>", "paragraph HTML changed");
        ensure!(restored[0]["elements"][0]["elements"][2]["settings"]["image"]["url"] == "https://example.test/roundtrip.jpg", "image URL changed");
        ensure!(restored[0]["elements"][0]["elements"][3]["settings"]["link"]["url"] == "https://example.test/apply", "button URL changed");
        ensure!(restored[0]["elements"][0]["elements"][3]["settings"]["link"]["is_external"] == true, "button target changed");
        ensure!(restored[0]["settings"]["css_classes"] == "mcp-native-section", "section class changed across native conversion");
        ensure!(restored[0]["elements"][0]["settings"]["css_classes"] == "mcp-native-column", "column class changed across native conversion");
        ensure!(restored[0]["elements"][0]["settings"]["_column_size_tablet"] == 75, "tablet layout width changed across native conversion");

        let dynamic_page = wp.post("wp/v2/pages", &json!({"title":format!("MCP {marker} dynamic"),"status":"draft"})).await.context("create dynamic binding retention fixture")?;
        let id = dynamic_page["id"].as_u64().context("WordPress omitted dynamic fixture ID")?;
        auxiliary_page_ids.push(id);
        let dynamic_source = save_dynamic_fixture(id, user_id).context("save native dynamic binding fixture")?;
        let dynamic_tree = get_page_document(&wp, id).await.context("read dynamic binding fixture")?;
        let dynamic_saved: Value = serde_json::from_str(&dynamic_tree.raw).context("dynamic source document is JSON")?;
        let dynamic_plan = ConvertElements.run(json!({"page_id":id,"operation_id":format!("{operation_id}_dynamic"),"direction":"classic_to_atomic"}), &wp).await.context("plan conversion for dynamic binding fixture")?;
        ensure!(dynamic_plan.is_error != Some(true), "dynamic conversion planning failed: {}", serde_json::to_string(&dynamic_plan)?);
        let dynamic_plan: Value = serde_json::from_str(serde_json::to_value(&dynamic_plan)?["content"][0]["text"].as_str().context("dynamic conversion returned text")?).context("dynamic conversion plan is JSON")?;
        ensure!(dynamic_plan["document_patch"].is_null(), "dynamic source unexpectedly produced a writable partial document: {dynamic_plan}");
        ensure!(dynamic_plan["retained"].as_array().is_some_and(|retained| retained.iter().any(|entry| entry["element_id"] == "7200001")), "dynamic widget subtree was silently discarded instead of retained: {dynamic_plan}");
        ensure!(dynamic_saved == dynamic_source, "native dynamic source changed before conversion planning");

        let runtime = wp.get("mcp-for-page-builders/v1/element-schemas").await.context("read native conversion renderer capabilities")?;
        let attributes_available = runtime.pointer("/render_capabilities/attributes/available").and_then(Value::as_bool);
        let classic_attributes_available = runtime.pointer("/widgets/button/controls/_attributes").is_some();
        let attributes_source = json!([
            {"id":"7300001","elType":"widget","widgetType":"button","settings":{"text":"Open details","link":{"url":"https://example.test/details","is_external":false},"_attributes":"data-mcp-native|attribute-value"},"elements":[]}
        ]);
        let attributes_page = wp.post("wp/v2/pages", &json!({"title":format!("MCP {marker} attributes"),"status":"draft","meta":{"_elementor_data":serde_json::to_string(&attributes_source)? ,"_elementor_edit_mode":"builder"}})).await.context("create native attributes fixture")?;
        let attributes_page_id = attributes_page["id"].as_u64().context("WordPress omitted attributes fixture ID")?;
        auxiliary_page_ids.push(attributes_page_id);
        let attributes_before = get_page_document(&wp, attributes_page_id).await.context("read native attributes source")?;
        let attributes_saved: Value = serde_json::from_str(&attributes_before.raw).context("attributes source is JSON")?;
        ensure!(attributes_saved == attributes_source, "native source save changed custom attributes before conversion");
        let attributes_plan = ConvertElements.run(json!({"page_id":attributes_page_id,"operation_id":format!("{operation_id}_attributes"),"direction":"classic_to_atomic"}), &wp).await.context("plan native attributes conversion")?;
        ensure!(attributes_plan.is_error != Some(true), "attributes conversion planning failed: {}", serde_json::to_string(&attributes_plan)?);
        let attributes_plan: Value = serde_json::from_str(serde_json::to_value(&attributes_plan)?["content"][0]["text"].as_str().context("attributes conversion returned text")?).context("attributes conversion plan is JSON")?;
        if attributes_available == Some(true) {
            ensure!(attributes_plan["retained"].as_array().is_some_and(Vec::is_empty) && attributes_plan["document_patch"].is_array(), "verified native attributes capability did not convert the complete widget: {attributes_plan}");
            let applied = ApplyElementorPlan.run(json!({"plan":attributes_plan}), &wp).await.context("save native Atomic attributes conversion")?;
            ensure!(applied.is_error != Some(true), "native Atomic attributes save failed: {}", serde_json::to_string(&applied)?);
            let atomic_doc = get_page_document(&wp, attributes_page_id).await.context("reload native Atomic attributes")?;
            let atomic: Value = serde_json::from_str(&atomic_doc.raw).context("saved Atomic attributes are JSON")?;
            ensure!(atomic[0]["widgetType"] == "e-button" && atomic[0]["settings"]["attributes"].to_string().contains("data-mcp-native") && atomic[0]["settings"]["attributes"].to_string().contains("attribute-value"), "native save lost typed Atomic attributes: {atomic}");
            let rendered = native_rendered_html(attributes_page_id)?;
            ensure!(rendered.contains("data-mcp-native=\"attribute-value\""), "verified Atomic attribute capability did not match native rendered HTML: {rendered}");
            let reverse = ConvertElements.run(json!({"page_id":attributes_page_id,"operation_id":format!("{operation_id}_attributes_reverse"),"direction":"atomic_to_classic"}), &wp).await.context("plan native attributes reverse conversion")?;
            ensure!(reverse.is_error != Some(true), "attributes reverse planning failed: {}", serde_json::to_string(&reverse)?);
            let reverse: Value = serde_json::from_str(serde_json::to_value(&reverse)?["content"][0]["text"].as_str().context("attributes reverse conversion returned text")?).context("attributes reverse plan is JSON")?;
            if classic_attributes_available {
                ensure!(reverse["retained"].as_array().is_some_and(Vec::is_empty) && reverse["document_patch"].is_array(), "native Classic attribute control did not support reverse conversion: {reverse}");
                let applied = ApplyElementorPlan.run(json!({"plan":reverse}), &wp).await.context("save native Classic attributes conversion")?;
                ensure!(applied.is_error != Some(true), "native Classic attributes save failed: {}", serde_json::to_string(&applied)?);
                let restored = get_page_document(&wp, attributes_page_id).await.context("reload native Classic attributes")?;
                let restored: Value = serde_json::from_str(&restored.raw).context("restored Classic attributes are JSON")?;
                ensure!(restored[0]["settings"]["_attributes"] == "data-mcp-native|attribute-value", "native reverse save changed Classic custom attributes: {restored}");
            } else {
                ensure!(reverse["document_patch"].is_null() && reverse["retained"].as_array().is_some_and(|retained| retained.iter().any(|entry| entry["element_id"] == "7300001")), "missing Classic attribute control did not retain the complete Atomic subtree: {reverse}");
            }
        } else {
            ensure!(attributes_plan["document_patch"].is_null() && attributes_plan["retained"].as_array().is_some_and(|retained| retained.iter().any(|entry| entry["element_id"] == "7300001")), "unknown or unavailable Atomic attribute renderer did not retain the complete source subtree: {attributes_plan}");
            let after = get_page_document(&wp, attributes_page_id).await.context("verify unsupported attribute source remains unchanged")?;
            ensure!(after.raw == attributes_before.raw, "unsupported attributes conversion changed the source document");
        }
        Ok(())
    }.await;
    let mut auxiliary_cleanup = Vec::new();
    for id in auxiliary_page_ids {
        let deleted = wp.delete(&format!("wp/v2/pages/{id}?force=true")).await;
        let absent = wp
            .get(&format!("wp/v2/pages/{id}?context=edit"))
            .await
            .is_err();
        auxiliary_cleanup.push((id, deleted, absent));
    }
    let deleted = wp
        .delete(&format!("wp/v2/pages/{page_id}?force=true"))
        .await;
    for (id, auxiliary_deleted, absent) in auxiliary_cleanup {
        assert!(
            auxiliary_deleted
                .as_ref()
                .is_ok_and(|value| value["deleted"] == true)
                && absent,
            "failed to clean exact auxiliary fixture page {id}: delete={auxiliary_deleted:?}, absent={absent}"
        );
    }
    assert!(
        deleted.as_ref().is_ok_and(|value| value["deleted"] == true),
        "failed to delete exact native roundtrip draft {page_id}: {deleted:?}; test error: {}",
        run.as_ref()
            .err()
            .map(ToString::to_string)
            .unwrap_or_else(|| "none".into())
    );
    assert!(
        wp.get(&format!("wp/v2/pages/{page_id}?context=edit"))
            .await
            .is_err(),
        "native roundtrip draft {page_id} remains after cleanup"
    );
    run.unwrap_or_else(|error| {
        panic!("native serialization roundtrip failed for draft {page_id}: {error:#}")
    });
}

fn cleanup_selected_conversion_receipts(
    page_id: u64,
    user_id: u64,
    operation_ids: &[String],
) -> anyhow::Result<()> {
    if operation_ids.is_empty() {
        return Ok(());
    }
    let ids = serde_json::to_string(operation_ids)?;
    let php = format!(
        "$page_id = {page_id}; $user_id = {user_id}; $operation_ids = json_decode('{ids}', true, 512, JSON_THROW_ON_ERROR); $removed = 0; foreach ($operation_ids as $operation_id) {{ if (!is_string($operation_id) || !preg_match('/^[A-Za-z0-9_-]{{8,128}}$/', $operation_id)) throw new RuntimeException('invalid owned operation ID'); $document_key = '_mcp_elementor_operation_' . hash('sha256', $operation_id); $document_receipt = get_option($document_key, null); if ($document_receipt !== null) {{ if (!is_array($document_receipt) || (int) ($document_receipt['post_id'] ?? 0) !== $page_id || (int) ($document_receipt['user_id'] ?? 0) !== $user_id) throw new RuntimeException('document receipt does not belong to this exact fixture'); $title = $document_receipt['before_state']['post']['post_title'] ?? null; if (!is_string($title) || !preg_match('/^MCP (selected_conversion_[0-9]+_[0-9]+)$/D', $title, $matches) || $operation_id !== $matches[1] . '_forward' && $operation_id !== $matches[1] . '_reverse') throw new RuntimeException('document receipt marker does not match its exact operation ID'); foreach (['original_document', 'applied_document'] as $field) {{ if (!is_string($document_receipt[$field] ?? null)) throw new RuntimeException('owned document receipt omitted a document snapshot'); foreach (['8100001', '8100003', '8100004', '8100005', '8100006', '8100007', '8100008', '8100009', '8100010'] as $element_id) {{ if (!str_contains($document_receipt[$field], '\"' . $element_id . '\"')) throw new RuntimeException('document receipt is not bound to this fixture document'); }} }} if (!delete_option($document_key) || get_option($document_key, null) !== null) throw new RuntimeException('could not delete and verify the exact owned document receipt'); $removed++; }} $resource_key = '_mcp_elementor_resources_' . hash('sha256', $operation_id); $resource_receipt = get_option($resource_key, null); if ($resource_receipt !== null) {{ if (!is_array($resource_receipt) || (int) ($resource_receipt['source_page_id'] ?? 0) !== $page_id || (int) ($resource_receipt['user_id'] ?? 0) !== $user_id || empty($resource_receipt['document_snapshot'])) throw new RuntimeException('resource receipt does not belong to this exact fixture'); $title = $resource_receipt['source_before_resources']['post']['post_title'] ?? null; if (!is_string($title) || !preg_match('/^MCP (selected_conversion_[0-9]+_[0-9]+)$/D', $title, $matches) || $operation_id !== $matches[1] . '_forward' && $operation_id !== $matches[1] . '_reverse') throw new RuntimeException('resource receipt marker does not match its exact operation ID'); $source_meta = $resource_receipt['source_before_resources']['meta'] ?? null; if (!is_array($source_meta)) throw new RuntimeException('resource receipt omitted the source metadata snapshot'); $source_document = null; foreach ($source_meta as $meta) if (($meta['meta_key'] ?? null) === '_elementor_data' && is_string($meta['meta_value'] ?? null)) $source_document = $meta['meta_value']; if (!is_string($source_document)) throw new RuntimeException('resource receipt omitted the source document'); foreach (['8100001', '8100003', '8100004', '8100005', '8100006', '8100007', '8100008', '8100009', '8100010'] as $element_id) {{ if (!str_contains($source_document, '\"' . $element_id . '\"')) throw new RuntimeException('resource receipt source IDs do not match this fixture'); }} if (!delete_option($resource_key) || get_option($resource_key, null) !== null) throw new RuntimeException('could not delete and verify the exact owned resource receipt'); $removed++; }} }} echo 'MCP_SELECTED_RECEIPTS:' . $removed;"
    );
    let output = Command::new("docker")
        .args([
            "exec",
            "-u",
            config().docker_user.as_str(),
            config().container.as_str(),
            "wp",
            config().wp_path_arg.as_str(),
            "eval",
            &php,
        ])
        .output()
        .context("clean exact selected-conversion operation receipts")?;
    ensure!(
        output.status.success(),
        "could not verify and clean exact selected-conversion operation receipts: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout)?;
    ensure!(
        stdout.contains("MCP_SELECTED_RECEIPTS:"),
        "owned operation receipt cleanup omitted its verification marker"
    );
    Ok(())
}

fn find_element<'a>(nodes: &'a Value, id: &str) -> Option<&'a Value> {
    for node in nodes.as_array()? {
        if node["id"] == id {
            return Some(node);
        }
        if let Some(found) = find_element(&node["elements"], id) {
            return Some(found);
        }
    }
    None
}

fn element_id_order(nodes: &Value) -> Vec<String> {
    let mut ids = Vec::new();
    if let Some(nodes) = nodes.as_array() {
        for node in nodes {
            if let Some(id) = node["id"].as_str() {
                ids.push(id.to_owned());
            }
            ids.extend(element_id_order(&node["elements"]));
        }
    }
    ids
}

fn json_differences(before: &Value, after: &Value) -> Vec<String> {
    fn collect(before: &Value, after: &Value, path: &str, differences: &mut Vec<String>) {
        if before == after || differences.len() >= 20 {
            return;
        }
        match (before, after) {
            (Value::Object(before), Value::Object(after)) => {
                let keys: BTreeSet<_> = before.keys().chain(after.keys()).collect();
                for key in keys {
                    if differences.len() >= 20 {
                        return;
                    }
                    let child = format!("{path}/{key}");
                    match (before.get(key), after.get(key)) {
                        (Some(before), Some(after)) => collect(before, after, &child, differences),
                        (Some(before), None) => {
                            differences.push(format!("{child} removed: {before}"))
                        }
                        (None, Some(after)) => differences.push(format!("{child} added: {after}")),
                        (None, None) => unreachable!(),
                    }
                }
            }
            (Value::Array(before), Value::Array(after)) => {
                if before.len() != after.len() {
                    differences.push(format!("{path} length {} -> {}", before.len(), after.len()));
                }
                for (index, (before, after)) in before.iter().zip(after).enumerate() {
                    if differences.len() >= 20 {
                        return;
                    }
                    collect(before, after, &format!("{path}/{index}"), differences);
                }
            }
            _ => differences.push(format!("{path}: {before} -> {after}")),
        }
    }

    let mut differences = Vec::new();
    collect(before, after, "", &mut differences);
    differences
}

fn normalize_native_serializer_defaults(document: &Value) -> Value {
    fn visit(node: &mut Value) {
        let element_type = node["elType"].as_str().map(str::to_owned);
        if !matches!(
            element_type.as_deref(),
            Some("widget" | "section" | "column" | "container" | "wrapper" | "e-flexbox")
        ) {
            return;
        }
        if element_type.as_deref() == Some("widget")
            && (node["isInner"] == false || node["isInner"].is_null())
        {
            if let Some(fields) = node.as_object_mut() {
                fields.remove("isInner");
            }
        }
        if matches!(element_type.as_deref(), Some("section" | "column")) && node["isInner"] == false
        {
            if let Some(fields) = node.as_object_mut() {
                fields.remove("isInner");
            }
        }
        for field in ["interactions", "editor_settings"] {
            if node[field].as_array().is_some_and(Vec::is_empty) {
                if let Some(fields) = node.as_object_mut() {
                    fields.remove(field);
                }
            }
        }
        if let Some(styles) = node.get_mut("styles").and_then(Value::as_object_mut) {
            for (id, style) in styles {
                if style["id"].as_str() != Some(id.as_str()) || style["type"] != "class" {
                    continue;
                }
                let Some(variants) = style.get_mut("variants").and_then(Value::as_array_mut) else {
                    continue;
                };
                for variant in variants {
                    let valid_native_variant = variant["meta"].is_object()
                        && matches!(
                            variant["meta"]["breakpoint"].as_str(),
                            Some("desktop" | "tablet" | "mobile")
                        )
                        && (variant["meta"]["state"].is_null()
                            || variant["meta"]["state"].is_string())
                        && variant["props"].is_object();
                    if valid_native_variant && variant["custom_css"].is_null() {
                        if let Some(fields) = variant.as_object_mut() {
                            fields.remove("custom_css");
                        }
                    }
                }
            }
        }
        if let Some(children) = node.get_mut("elements").and_then(Value::as_array_mut) {
            for child in children {
                visit(child);
            }
        }
    }

    let mut normalized = document.clone();
    if let Some(nodes) = normalized.as_array_mut() {
        for node in nodes {
            visit(node);
        }
    }
    normalized
}

fn style_map_order(document: &Value) -> Vec<(String, Vec<String>)> {
    fn visit(nodes: &Value, styles: &mut Vec<(String, Vec<String>)>) {
        let Some(nodes) = nodes.as_array() else {
            return;
        };
        for node in nodes {
            if let Some(style_map) = node.get("styles").and_then(Value::as_object) {
                styles.push((
                    node["id"].as_str().unwrap_or_default().to_owned(),
                    style_map.keys().cloned().collect(),
                ));
            }
            visit(&node["elements"], styles);
        }
    }

    let mut styles = Vec::new();
    visit(document, &mut styles);
    styles
}

#[test]
fn native_serializer_default_normalization_keeps_explicit_values_and_order() -> anyhow::Result<()> {
    let source = json!([
        {"id":"8100001","elType":"widget","isInner":false,"interactions":[],"editor_settings":[],"settings":{"isInner":false,"interactions":[],"keep_null":null,"classes":{"$$type":"classes","value":["first","second"]}},"styles":{"style-a":{"id":"style-a","type":"class","variants":[{"meta":{"breakpoint":"desktop","state":null},"props":{"color":{"$$type":"color","value":"#123456"}},"custom_css":"selector { color: red; }","keep_null":null}]},"invalid":{"id":"different","type":"class","variants":[{"custom_css":null}] }},"elements":[]},
        {"id":"8100002","elType":"e-flexbox","isInner":false,"settings":{},"styles":{"mcp-layout-a":{"id":"mcp-layout-a","type":"class","variants":[{"meta":{"breakpoint":"desktop","state":null},"props":{"display":{"$$type":"string","value":"flex"},"flex-direction":{"$$type":"string","value":"row"}},"custom_css":null,"other_null":null}] }},"elements":[]},
        {"id":"8100003","elType":"widget","isInner":true,"interactions":[{"type":"click"}],"editor_settings":{"panel":"open"},"settings":{},"elements":[]},
        {"id":"8100004","elType":"widget","isInner":null,"settings":{},"elements":[]},
        {"id":"8100005","elType":"section","isInner":false,"elements":[{"id":"8100006","elType":"column","isInner":false,"settings":{"custom_payload":{"isInner":false,"editor_settings":[]}},"elements":[]}]},
        {"id":"8100007","elType":"e-flexbox","isInner":false,"elements":[]},
        {"id":"8100008","elType":"unknown-native-kind","interactions":[],"editor_settings":[],"isInner":false,"elements":[{"id":"8100009","elType":"widget","isInner":false,"elements":[]}]}
    ]);
    let normalized = normalize_native_serializer_defaults(&source);
    ensure!(
        normalized[0].get("isInner").is_none(),
        "native widget default isInner=false must be omitted"
    );
    ensure!(
        normalized[0]["settings"]["isInner"] == false
            && normalized[0]["settings"]["keep_null"].is_null(),
        "normalization changed a settings value or unrelated null"
    );
    ensure!(
        normalized[0]["settings"]["classes"]["value"] == json!(["first", "second"]),
        "normalization reordered class bindings"
    );
    ensure!(
        normalized[0]["styles"]["style-a"]["variants"][0]["custom_css"]
            == "selector { color: red; }",
        "normalization removed non-null custom CSS"
    );
    ensure!(
        normalized[0]["styles"]["style-a"]["variants"][0]["keep_null"].is_null(),
        "normalization removed an unrelated null field"
    );
    ensure!(
        normalized[0]["styles"]["invalid"]["variants"][0]["custom_css"].is_null(),
        "normalization changed a non-native style variant"
    );
    ensure!(
        normalized[1]["isInner"] == false
            && normalized[1]["styles"]["mcp-layout-a"]["variants"][0]
                .get("custom_css")
                .is_none(),
        "normalization did not isolate native style defaults"
    );
    ensure!(
        normalized[1]["styles"]["mcp-layout-a"]["variants"][0]["props"]
            == source[1]["styles"]["mcp-layout-a"]["variants"][0]["props"]
            && normalized[1]["styles"]["mcp-layout-a"]["variants"][0]["other_null"].is_null(),
        "normalization changed style declarations or unrelated null fields"
    );
    ensure!(
        normalized[2]["isInner"] == true,
        "normalization changed isInner=true"
    );
    ensure!(
        normalized[0].get("interactions").is_none()
            && normalized[0].get("editor_settings").is_none()
            && normalized[0]["settings"]["interactions"] == json!([]),
        "normalization did not limit empty native arrays to element-level fields"
    );
    ensure!(
        normalized[2]["interactions"] == json!([{"type":"click"}])
            && normalized[2]["editor_settings"] == json!({"panel":"open"}),
        "normalization changed nonempty native element metadata"
    );
    ensure!(
        normalized[3].get("isInner").is_none()
            && normalized[4].get("isInner").is_none()
            && normalized[4]["elements"][0].get("isInner").is_none()
            && normalized[5]["isInner"] == false,
        "normalization did not isolate widget/Classic section and column defaults from e-flexbox"
    );
    ensure!(
        normalized[4]["elements"][0]["settings"]["custom_payload"]
            == source[4]["elements"][0]["settings"]["custom_payload"]
            && normalized[6] == source[6],
        "normalization recursed into custom payload or an unknown element type"
    );
    ensure!(
        style_map_order(&normalized) == style_map_order(&source),
        "normalization changed native style-map key order"
    );
    Ok(())
}

#[tokio::test]
async fn selected_conversion_saves_only_requested_native_subtrees_and_reverses_them()
-> anyhow::Result<()> {
    let config = config();
    let wp = WpClient::new(&config.url, &config.user, &config.password);
    let current_user = wp
        .get("wp/v2/users/me?context=edit")
        .await
        .expect("read authenticated test user");
    let user_id = current_user["id"].as_u64().expect("authenticated user ID");
    let marker = format!(
        "selected_conversion_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock is after the Unix epoch")
            .as_nanos()
    );
    let created = wp
        .post(
            "wp/v2/pages",
            &json!({"title":format!("MCP {marker}"),"status":"draft","meta":{"_elementor_data":"[]","_elementor_edit_mode":"builder"}}),
        )
        .await
        .expect("create exact selected-conversion fixture draft");
    let page_id = created["id"].as_u64().expect("fixture page ID");
    let mut applied_operation_ids = Vec::new();
    let run: anyhow::Result<()> = async {
        let source = save_selected_fixture(page_id, user_id)?;
        let before = get_page_document(&wp, page_id)
            .await
            .context("read native selected-conversion source")?;
        let source_readback: Value = serde_json::from_str(&before.raw)?;
        ensure!(source_readback == source, "native selected fixture differs from its save readback");
        ensure!(source[2]["settings"]["__dynamic__"]["title"].is_string(), "unsupported sibling lacks its registered dynamic binding");
        ensure!(source[4]["id"] == "8100010" && source[4]["settings"]["link"]["nofollow"] == "yes", "nofollow negative fixture was not preserved by native Classic save");
        let button_schema = ElementorService::new(&wp)
            .element_schema("e-button")
            .await
            .context("read native Atomic button link schema")?;
        let link_fields = button_schema
            .pointer("/props_schema/link/prop_types/link/shape")
            .and_then(Value::as_object)
            .context("native Atomic button link schema omitted its typed fields")?
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        ensure!(link_fields == ["destination", "isTargetBlank", "tag"].into_iter().map(str::to_owned).collect(), "native Atomic button schema changed; recheck whether it can represent Classic nofollow: {link_fields:?}");

        for (case, ids) in [
            ("missing", json!(["8199999"])),
            ("duplicate", json!(["8100003", "8100003"])),
            ("parent_child_overlap", json!(["8100001", "8100003"])),
            ("valid_plus_missing", json!(["8100003", "8199999"])),
        ] {
            let rejected = ConvertElements
                .run(
                    json!({"page_id":page_id,"operation_id":format!("{marker}_{case}"),"direction":"classic_to_atomic","element_ids":ids}),
                    &wp,
                )
                .await;
            ensure!(
                rejected.is_err()
                    || rejected
                        .as_ref()
                        .is_ok_and(|response| response.is_error == Some(true)),
                "invalid selected IDs unexpectedly produced an executable plan ({case})"
            );
            let unchanged = get_page_document(&wp, page_id).await?;
            ensure!(
                unchanged.raw == before.raw && unchanged.revision == before.revision,
                "invalid selected IDs changed the native source ({case})"
            );
        }

        let unsupported = ConvertElements
            .run(
                json!({"page_id":page_id,"operation_id":format!("{marker}_dynamic"),"direction":"classic_to_atomic","element_ids":["8100007"]}),
                &wp,
            )
            .await?;
        ensure!(unsupported.is_error != Some(true), "unsupported target selection was not reported as a retained plan");
        let unsupported: Value = serde_json::from_str(
            serde_json::to_value(&unsupported)?["content"][0]["text"]
                .as_str()
                .context("unsupported selection returned no plan text")?,
        )?;
        ensure!(unsupported["document_patch"].is_null(), "unsupported dynamic subtree produced a partial write patch: {unsupported}");
        ensure!(unsupported["retained"].as_array().is_some_and(|retained| retained.iter().any(|entry| entry["element_id"] == "8100007")), "unsupported dynamic subtree was not retained: {unsupported}");

        let nofollow = ConvertElements
            .run(
                json!({"page_id":page_id,"operation_id":format!("{marker}_nofollow"),"direction":"classic_to_atomic","element_ids":["8100010"]}),
                &wp,
            )
            .await?;
        ensure!(nofollow.is_error != Some(true), "nofollow negative selection did not produce a retained plan");
        let nofollow: Value = serde_json::from_str(
            serde_json::to_value(&nofollow)?["content"][0]["text"]
                .as_str()
                .context("nofollow selection returned no plan text")?,
        )?;
        ensure!(nofollow["document_patch"].is_null(), "unsupported nofollow link produced an executable partial patch: {nofollow}");
        ensure!(nofollow["retained"].as_array().is_some_and(|retained| retained.iter().any(|entry| entry["element_id"] == "8100010" && entry["reason"] == "matching active Atomic prop schema or lossless field mapping is unavailable")), "unsupported nofollow link did not retain its complete button subtree with the expected diagnostic: {nofollow}");
        let unchanged = get_page_document(&wp, page_id).await?;
        ensure!(unchanged.raw == before.raw && unchanged.revision == before.revision, "unsupported nofollow link changed the native source");

        let planned = ConvertElements
            .run(
                json!({"page_id":page_id,"operation_id":format!("{marker}_forward"),"direction":"classic_to_atomic","element_ids":["8100001","8100008"]}),
                &wp,
            )
            .await?;
        ensure!(planned.is_error != Some(true), "selected Classic conversion planning failed");
        let plan: Value = serde_json::from_str(
            serde_json::to_value(&planned)?["content"][0]["text"]
                .as_str()
                .context("selected conversion returned no plan text")?,
        )?;
        ensure!(plan["source"]["selection_scope"] == "selected_disjoint_subtrees", "plan did not identify its selected scope");
        ensure!(plan["source"]["selected_element_ids"] == json!(["8100001", "8100008"]), "plan changed selected IDs");
        ensure!(plan["document_patch"].is_array(), "selected conversion did not produce a complete executable patch: retained={}", plan["retained"]);
        let patch = plan["document_patch"].as_array().context("selected plan is a full document patch")?;
        ensure!(patch.len() == source.as_array().context("source is a native document array")?.len(), "selected patch omitted unselected top-level siblings");
        let patch_value = Value::Array(patch.clone());
        ensure!(element_id_order(&patch_value) == element_id_order(&source), "selected plan changed the complete document ID/order list");
        for (id, field, kind) in [
            ("8100001", "elType", "e-flexbox"),
            ("8100003", "widgetType", "e-heading"),
            ("8100004", "widgetType", "e-button"),
            ("8100005", "widgetType", "e-image"),
            ("8100008", "elType", "e-flexbox"),
            ("8100009", "widgetType", "e-heading"),
        ] {
            ensure!(find_element(&patch_value, id).is_some_and(|node| node[field] == kind), "selected subtree node {id} was not converted to {kind}");
        }
        for id in ["8100006", "8100007", "8100010"] {
            ensure!(find_element(&patch_value, id) == find_element(&source, id), "selected patch changed unselected subtree {id}");
        }

        let forward_revisions_before = native_revision_ids(page_id)?;
        applied_operation_ids.push(format!("{marker}_forward"));
        let applied = ApplyElementorPlan
            .run(json!({"plan":plan}), &wp)
            .await
            .context("apply selected native conversion")?;
        ensure!(applied.is_error != Some(true), "selected native conversion save failed: {}", serde_json::to_string(&applied)?);
        let forward_revisions_after = native_revision_ids(page_id)?;
        ensure!(forward_revisions_after.len() > forward_revisions_before.len(), "public ApplyElementorPlan did not create native Elementor revisions: before={forward_revisions_before:?}, after={forward_revisions_after:?}");
        let saved = get_page_document(&wp, page_id).await?;
        let atomic_after_plan: Value = serde_json::from_str(&saved.raw)?;
        let atomic_roundtrip = native_resave_document(page_id, user_id, &atomic_after_plan)?;
        let atomic_evidence = save_native_roundtrip_evidence(
            page_id,
            "atomic",
            &atomic_after_plan,
            &atomic_roundtrip,
        )?;
        ensure!(normalize_native_serializer_defaults(&atomic_roundtrip) == normalize_native_serializer_defaults(&atomic_after_plan), "native Elementor Document::save changed the selected Atomic document: {:?}", json_differences(&normalize_native_serializer_defaults(&atomic_after_plan), &normalize_native_serializer_defaults(&atomic_roundtrip)));
        ensure!(style_map_order(&atomic_roundtrip) == style_map_order(&atomic_after_plan), "native Elementor Document::save changed local style-map declaration order");
        let saved = get_page_document(&wp, page_id).await?;
        let atomic: Value = serde_json::from_str(&saved.raw)?;
        ensure!(element_id_order(&atomic) == element_id_order(&source), "native save changed the complete document ID/order list");
        ensure!(find_element(&atomic, "8100001").is_some_and(|node| node["elType"] == "e-flexbox"), "native reload lost selected Atomic layout");
        ensure!(find_element(&atomic, "8100004").is_some_and(|node| {
            node.pointer("/settings/link/value/destination/value") == Some(&json!("https://example.test/apply"))
                && node.pointer("/settings/link/value/isTargetBlank/value") == Some(&json!(true))
        }), "native save lost the selected button destination or target flag");
        ensure!(find_element(&atomic, "8100008").is_some_and(|node| node["elType"] == "e-flexbox") && find_element(&atomic, "8100009").is_some_and(|node| node["widgetType"] == "e-heading"), "native save changed selected wrapper conversion state");
        for id in ["8100006", "8100007", "8100010"] {
            ensure!(find_element(&atomic, id) == find_element(&source, id), "native save changed unselected subtree {id}");
        }
        let rendered = native_rendered_html(page_id)?;
        for expected in ["Selected heading", "Unselected heading", "Wrapper child", "https://example.test/apply", "https://example.test/selected.jpg"] {
            ensure!(rendered.contains(expected), "native renderer omitted {expected}: {rendered}");
        }

        let reverse = ConvertElements
            .run(
                json!({"page_id":page_id,"operation_id":format!("{marker}_reverse"),"direction":"atomic_to_classic","element_ids":["8100001"]}),
                &wp,
            )
            .await?;
        ensure!(reverse.is_error != Some(true), "selected Atomic reverse conversion planning failed");
        let reverse: Value = serde_json::from_str(
            serde_json::to_value(&reverse)?["content"][0]["text"]
                .as_str()
                .context("reverse selection returned no plan text")?,
        )?;
        ensure!(element_id_order(&reverse["document_patch"]) == element_id_order(&atomic), "reverse plan changed the complete document ID/order list");
        for id in ["8100006", "8100007", "8100008", "8100010"] {
            ensure!(find_element(&reverse["document_patch"], id) == find_element(&atomic, id), "reverse selected patch changed unselected subtree {id}");
        }
        let reverse_revisions_before = native_revision_ids(page_id)?;
        applied_operation_ids.push(format!("{marker}_reverse"));
        let applied = ApplyElementorPlan
            .run(json!({"plan":reverse}), &wp)
            .await
            .context("apply selected native reverse conversion")?;
        ensure!(applied.is_error != Some(true), "selected Classic reverse save failed: {}", serde_json::to_string(&applied)?);
        let reverse_revisions_after = native_revision_ids(page_id)?;
        ensure!(reverse_revisions_after.len() > reverse_revisions_before.len(), "public reverse ApplyElementorPlan did not create native Elementor revisions: before={reverse_revisions_before:?}, after={reverse_revisions_after:?}");
        let restored_doc = get_page_document(&wp, page_id).await?;
        let restored_before_native_save: Value = serde_json::from_str(&restored_doc.raw)?;
        let restored_roundtrip = native_resave_document(page_id, user_id, &restored_before_native_save)?;
        let classic_evidence = save_native_roundtrip_evidence(
            page_id,
            "classic",
            &restored_before_native_save,
            &restored_roundtrip,
        )?;
        ensure!(normalize_native_serializer_defaults(&restored_roundtrip) == normalize_native_serializer_defaults(&restored_before_native_save), "native Elementor Document::save changed the reversed Classic document: {:?}", json_differences(&normalize_native_serializer_defaults(&restored_before_native_save), &normalize_native_serializer_defaults(&restored_roundtrip)));
        ensure!(
            fs::metadata(&atomic_evidence)?.permissions().mode() & 0o777 == 0o600
                && fs::metadata(&classic_evidence)?.permissions().mode() & 0o777 == 0o600,
            "native serialized evidence files must remain private"
        );
        ensure!(style_map_order(&restored_roundtrip) == style_map_order(&restored_before_native_save), "native Classic Document::save changed style-map declaration order");
        let restored_doc = get_page_document(&wp, page_id).await?;
        let restored: Value = serde_json::from_str(&restored_doc.raw)?;
        ensure!(element_id_order(&restored) == element_id_order(&atomic), "native reverse save changed the complete document ID/order list");
        ensure!(find_element(&restored, "8100001").is_some_and(|node| node["elType"] == "section") && find_element(&restored, "8100002").is_some_and(|node| node["elType"] == "column"), "native reverse save lost Classic responsive layout nodes");
        ensure!(find_element(&restored, "8100002").is_some_and(|node| node["settings"]["_column_size_tablet"] == 75), "native reverse save lost the tablet column width");
        ensure!(find_element(&restored, "8100003").is_some_and(|node| node["settings"]["title"] == "Selected heading"), "native reverse save lost selected heading text");
        ensure!(find_element(&restored, "8100004").is_some_and(|node| node["settings"]["link"]["url"] == "https://example.test/apply" && node["settings"]["link"]["is_external"] == true), "native reverse save lost button control values");
        ensure!(find_element(&restored, "8100005").is_some_and(|node| node["settings"]["image"]["url"] == "https://example.test/selected.jpg" && node["settings"]["link"]["url"] == "https://example.test/media/"), "native reverse save lost image media or link values");
        for id in ["8100006", "8100007", "8100008", "8100010"] {
            ensure!(find_element(&restored, id) == find_element(&atomic, id), "native reverse save changed unselected subtree {id}");
        }
        let final_render = native_rendered_html(page_id)?;
        for expected in ["Selected heading", "Apply now", "https://example.test/apply", "https://example.test/selected.jpg"] {
            ensure!(final_render.contains(expected), "native Classic renderer omitted {expected}: {final_render}");
        }
        Ok(())
    }
    .await;
    let deleted = wp
        .delete(&format!("wp/v2/pages/{page_id}?force=true"))
        .await;
    ensure!(
        deleted
            .as_ref()
            .is_ok_and(|response| response["deleted"] == true)
            && wp
                .get(&format!("wp/v2/pages/{page_id}?context=edit"))
                .await
                .is_err(),
        "could not clean exact selected-conversion fixture {page_id}: {deleted:?}; test error: {}",
        run.as_ref()
            .err()
            .map(ToString::to_string)
            .unwrap_or_else(|| "none".into())
    );
    cleanup_selected_conversion_receipts(page_id, user_id, &applied_operation_ids).unwrap_or_else(
        |error| panic!("selected conversion receipt cleanup failed for draft {page_id}: {error:#}"),
    );
    run.unwrap_or_else(|error| {
        panic!("selected native conversion failed for draft {page_id}: {error:#}")
    });
    Ok(())
}
