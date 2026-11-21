package dev.slint.todowidget;

import android.appwidget.AppWidgetManager;
import android.appwidget.AppWidgetProvider;
import android.content.Context;
import android.content.Intent;

/** Thin shim: Android requires a class here, all the logic is in Rust. */
public class TodoWidgetProvider extends AppWidgetProvider {
	static {
		System.loadLibrary("todo_widget_lib");
	}

	@Override
	public void onUpdate(Context context, AppWidgetManager manager, int[] ids) {
		nativeUpdate(context, manager, ids);
	}

	@Override
	public void onReceive(Context context, Intent intent) {
		if ("dev.slint.todowidget.TOGGLE".equals(intent.getAction())) {
			nativeToggle(context, intent.getIntExtra("index", -1));
		} else {
			super.onReceive(context, intent);
		}
	}

	private static native void nativeToggle(Context context, int index);

	private static native void nativeUpdate(Context context, AppWidgetManager manager, int[] ids);
}
