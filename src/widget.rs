//! Home screen widget: renders the todo list with Slint's software renderer into a bitmap and
//! pushes it into the widget's `RemoteViews`. Entry points are `TodoWidgetProvider` (JNI) and
//! [`refresh_all`], which the app calls after it saved its todos.

use crate::{TodoItem, TodoWidget};
use jni::objects::{JClass, JIntArray, JObject, JValue};
use jni::sys::jint;
use jni::{JNIEnv, JavaVM};
use slint::platform::software_renderer::{
	MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::platform::{Platform, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, ModelRc, PhysicalSize, PlatformError, VecModel};
use std::path::{Path, PathBuf};
use std::rc::Rc;

type Result<T> = jni::errors::Result<T>;

const PROVIDER: &str = "dev.slint.todowidget.TodoWidgetProvider";
const MAX_ITEMS: i32 = 5;
pub const TODOS_FILE: &str = "todos.json";

struct WidgetPlatform(Rc<MinimalSoftwareWindow>);

impl Platform for WidgetPlatform {
	fn create_window_adapter(&self) -> std::result::Result<Rc<dyn WindowAdapter>, PlatformError> {
		Ok(self.0.clone())
	}
}

pub fn todos_path(files_dir: &Path) -> PathBuf {
	files_dir.join(TODOS_FILE)
}

fn load_items(files_dir: &Path) -> Vec<TodoItem> {
	std::fs::read(todos_path(files_dir))
		.ok()
		.and_then(|bytes| serde_json::from_slice(&bytes).ok())
		.unwrap_or_else(|| {
			vec![TodoItem { checked: false, title: "Open the app to add todos".into() }]
		})
}

/// Render the widget UI to premultiplied RGBA8888. Slint's platform is per thread, so this runs on
/// a short-lived thread: that works even when the app's own Android platform is set on another one.
fn render(items: Vec<TodoItem>, width: u32, height: u32, scale: f32) -> Vec<u8> {
	std::thread::spawn(move || {
		let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
		slint::platform::set_platform(Box::new(WidgetPlatform(window.clone()))).ok();
		let ui = TodoWidget::new().unwrap();
		ui.set_done_count(items.iter().filter(|i| i.checked).count() as i32);
		ui.set_items(ModelRc::new(VecModel::from(items)));
		ui.window().dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: scale });
		window.set_size(PhysicalSize::new(width, height));
		ui.show().unwrap();
		let mut buf = vec![PremultipliedRgbaColor::default(); (width * height) as usize];
		window.draw_if_needed(|renderer| {
			renderer.render(&mut buf, width as usize);
		});
		// Safety: `PremultipliedRgbaColor` is `repr(C)` with four `u8` fields.
		let bytes = unsafe {
			core::slice::from_raw_parts(buf.as_ptr() as *const u8, buf.len() * 4)
		};
		bytes.to_vec()
	})
	.join()
	.expect("render thread panicked")
}

fn make_bitmap<'l>(env: &mut JNIEnv<'l>, pixels: &[u8], w: i32, h: i32) -> Result<JObject<'l>> {
	let config_class = env.find_class("android/graphics/Bitmap$Config")?;
	let argb = env.get_static_field(config_class, "ARGB_8888", "Landroid/graphics/Bitmap$Config;")?.l()?;
	let bitmap_class = env.find_class("android/graphics/Bitmap")?;
	let bitmap = env
		.call_static_method(
			bitmap_class,
			"createBitmap",
			"(IILandroid/graphics/Bitmap$Config;)Landroid/graphics/Bitmap;",
			&[JValue::Int(w), JValue::Int(h), JValue::Object(&argb)],
		)?
		.l()?;
	// Safety: `pixels` outlives the buffer, which is only used for the copy below.
	let buffer = unsafe { env.new_direct_byte_buffer(pixels.as_ptr() as *mut u8, pixels.len())? };
	env.call_method(&bitmap, "copyPixelsFromBuffer", "(Ljava/nio/Buffer;)V", &[JValue::Object(&buffer)])?;
	Ok(bitmap)
}

fn identifier(env: &mut JNIEnv, res: &JObject, name: &str, kind: &str, package: &JObject) -> Result<i32> {
	let name = env.new_string(name)?;
	let kind = env.new_string(kind)?;
	env.call_method(
		res,
		"getIdentifier",
		"(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)I",
		&[JValue::Object(&name), JValue::Object(&kind), JValue::Object(package)],
	)?
	.i()
}

fn bundle_int(env: &mut JNIEnv, bundle: &JObject, key: &str, default: i32) -> Result<i32> {
	let key = env.new_string(key)?;
	env.call_method(bundle, "getInt", "(Ljava/lang/String;I)I", &[JValue::Object(&key), JValue::Int(default)])?
		.i()
}

fn files_dir(env: &mut JNIEnv, context: &JObject) -> Result<PathBuf> {
	let dir = env.call_method(context, "getFilesDir", "()Ljava/io/File;", &[])?.l()?;
	let path = env.call_method(&dir, "getAbsolutePath", "()Ljava/lang/String;", &[])?.l()?;
	let path: String = env.get_string(&path.into())?.into();
	Ok(path.into())
}

/// A broadcast to the provider that toggles the todo at `index`.
fn toggle_intent<'l>(env: &mut JNIEnv<'l>, context: &JObject, package: &JObject, index: i32) -> Result<JObject<'l>> {
	let intent = env.new_object("android/content/Intent", "()V", &[])?;
	let class = env.new_string(PROVIDER)?;
	env.call_method(&intent, "setClassName", "(Ljava/lang/String;Ljava/lang/String;)Landroid/content/Intent;", &[JValue::Object(package), JValue::Object(&class)])?;
	let action = env.new_string("dev.slint.todowidget.TOGGLE")?;
	env.call_method(&intent, "setAction", "(Ljava/lang/String;)Landroid/content/Intent;", &[JValue::Object(&action)])?;
	let key = env.new_string("index")?;
	env.call_method(&intent, "putExtra", "(Ljava/lang/String;I)Landroid/content/Intent;", &[JValue::Object(&key), JValue::Int(index % 16)])?;
	// FLAG_UPDATE_CURRENT | FLAG_IMMUTABLE; the request code keeps each target's intent distinct.
	let flags = 0x0800_0000 | 0x0400_0000;
	env.call_static_method(
		"android/app/PendingIntent",
		"getBroadcast",
		"(Landroid/content/Context;ILandroid/content/Intent;I)Landroid/app/PendingIntent;",
		&[JValue::Object(context), JValue::Int(index), JValue::Object(&intent), JValue::Int(flags)],
	)?
	.l()
}

fn update(env: &mut JNIEnv, context: &JObject, manager: &JObject, ids: &[i32]) -> Result<()> {
	let package = env.call_method(context, "getPackageName", "()Ljava/lang/String;", &[])?.l()?;
	let res = env.call_method(context, "getResources", "()Landroid/content/res/Resources;", &[])?.l()?;
	let layout = identifier(env, &res, "todo_widget", "layout", &package)?;
	let image_id = identifier(env, &res, "image", "id", &package)?;
	let metrics = env.call_method(&res, "getDisplayMetrics", "()Landroid/util/DisplayMetrics;", &[])?.l()?;
	let density = env.get_field(&metrics, "density", "F")?.f()?;
	let items = load_items(&files_dir(env, context)?);

	for &id in ids {
		let options = env
			.call_method(manager, "getAppWidgetOptions", "(I)Landroid/os/Bundle;", &[JValue::Int(id)])?
			.l()?;
		let w_dp = bundle_int(env, &options, "appWidgetMinWidth", 250)?.max(40);
		let h_dp = bundle_int(env, &options, "appWidgetMaxHeight", 150)?.max(40);
		let (w, h) = ((w_dp as f32 * density) as u32, (h_dp as f32 * density) as u32);
		let pixels = render(items.clone(), w, h, density);
		let bitmap = make_bitmap(env, &pixels, w as i32, h as i32)?;
		let views = env.new_object(
			"android/widget/RemoteViews",
			"(Ljava/lang/String;I)V",
			&[JValue::Object(&package), JValue::Int(layout)],
		)?;
		env.call_method(&views, "setImageViewBitmap", "(ILandroid/graphics/Bitmap;)V", &[JValue::Int(image_id), JValue::Object(&bitmap)])?;
		for row in 0..MAX_ITEMS {
			let row_id = identifier(env, &res, &format!("row{row}"), "id", &package)?;
			let pending = toggle_intent(env, context, &package, id * 16 + row)?;
			env.call_method(&views, "setOnClickPendingIntent", "(ILandroid/app/PendingIntent;)V", &[JValue::Int(row_id), JValue::Object(&pending)])?;
		}
		env.call_method(manager, "updateAppWidget", "(ILandroid/widget/RemoteViews;)V", &[JValue::Int(id), JValue::Object(&views)])?;
	}
	Ok(())
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_slint_todowidget_TodoWidgetProvider_nativeUpdate<'l>(
	mut env: JNIEnv<'l>,
	_class: JClass<'l>,
	context: JObject<'l>,
	manager: JObject<'l>,
	ids: JIntArray<'l>,
) {
	android_logger::init_once(android_logger::Config::default().with_tag("todo-widget"));
	let result = (|| {
		let len = env.get_array_length(&ids)? as usize;
		let mut widget_ids = vec![0i32; len];
		env.get_int_array_region(&ids, 0, &mut widget_ids)?;
		update(&mut env, &context, &manager, &widget_ids)
	})();
	if let Err(e) = result {
		let _ = env.exception_clear();
		log::error!("todo-widget: update failed: {e}");
	}
}

/// Re-render every widget instance, using the app's own context. Called by the app, e.g. after
/// saving its todos.
pub fn refresh_all() -> Result<()> {
	let ctx = ndk_context::android_context();
	// Safety: ndk-context provides a valid JavaVM and application/activity context.
	let vm = unsafe { JavaVM::from_raw(ctx.vm().cast())? };
	let mut env = vm.attach_current_thread()?;
	let context = unsafe { JObject::from_raw(ctx.context().cast()) };
	refresh_with(&mut env, &context)
}

fn refresh_with(env: &mut JNIEnv, context: &JObject) -> Result<()> {
	let manager = env
		.call_static_method(
			"android/appwidget/AppWidgetManager",
			"getInstance",
			"(Landroid/content/Context;)Landroid/appwidget/AppWidgetManager;",
			&[JValue::Object(context)],
		)?
		.l()?;
	let package = env.call_method(context, "getPackageName", "()Ljava/lang/String;", &[])?.l()?;
	let class = env.new_string(PROVIDER)?;
	let component = env.new_object(
		"android/content/ComponentName",
		"(Ljava/lang/String;Ljava/lang/String;)V",
		&[JValue::Object(&package), JValue::Object(&class)],
	)?;
	let ids = env
		.call_method(&manager, "getAppWidgetIds", "(Landroid/content/ComponentName;)[I", &[JValue::Object(&component)])?
		.l()?;
	let ids: JIntArray = ids.into();
	let len = env.get_array_length(&ids)? as usize;
	let mut widget_ids = vec![0i32; len];
	env.get_int_array_region(&ids, 0, &mut widget_ids)?;
	update(env, context, &manager, &widget_ids)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_slint_todowidget_TodoWidgetProvider_nativeToggle<'l>(
	mut env: JNIEnv<'l>,
	_class: JClass<'l>,
	context: JObject<'l>,
	index: jint,
) {
	android_logger::init_once(android_logger::Config::default().with_tag("todo-widget"));
	let result = (|| {
		let dir = files_dir(&mut env, &context)?;
		let mut items = load_items(&dir);
		if let Some(item) = usize::try_from(index).ok().and_then(|i| items.get_mut(i)) {
			item.checked = !item.checked;
			if let Err(e) = std::fs::write(todos_path(&dir), serde_json::to_vec(&items).unwrap_or_default()) {
				log::error!("cannot save todos: {e}");
			}
		}
		refresh_with(&mut env, &context)
	})();
	if let Err(e) = result {
		let _ = env.exception_clear();
		log::error!("todo-widget: toggle failed: {e}");
	}
}
