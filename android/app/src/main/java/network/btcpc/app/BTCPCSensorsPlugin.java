package network.btcpc.app;

import android.content.Context;
import android.hardware.Sensor;
import android.hardware.SensorEvent;
import android.hardware.SensorEventListener;
import android.hardware.SensorManager;

import com.getcapacitor.JSArray;
import com.getcapacitor.JSObject;
import com.getcapacitor.Plugin;
import com.getcapacitor.PluginCall;
import com.getcapacitor.PluginMethod;
import com.getcapacitor.annotation.CapacitorPlugin;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

@CapacitorPlugin(name = "BTCPCSensors")
public class BTCPCSensorsPlugin extends Plugin implements SensorEventListener {
  private SensorManager sensorManager;
  private final Map<Integer, String> sensorNames = new HashMap<>();
  private boolean running = false;

  @Override
  public void load() {
    sensorManager = (SensorManager) getContext().getSystemService(Context.SENSOR_SERVICE);
  }

  @PluginMethod
  public void list(PluginCall call) {
    JSArray sensors = new JSArray();
    if (sensorManager == null) {
      JSObject ret = new JSObject();
      ret.put("sensors", sensors);
      call.resolve(ret);
      return;
    }

    List<Sensor> all = sensorManager.getSensorList(Sensor.TYPE_ALL);
    for (Sensor sensor : all) {
      JSObject item = new JSObject();
      item.put("type", sensor.getType());
      item.put("name", sensor.getName());
      item.put("vendor", sensor.getVendor());
      item.put("btcpcId", mapSensorId(sensor.getType()));
      sensors.put(item);
    }

    JSObject ret = new JSObject();
    ret.put("sensors", sensors);
    call.resolve(ret);
  }

  @PluginMethod
  public void start(PluginCall call) {
    if (sensorManager == null) {
      call.reject("SensorManager unavailable");
      return;
    }
    stopSensors();
    List<Sensor> all = sensorManager.getSensorList(Sensor.TYPE_ALL);
    for (Sensor sensor : all) {
      String id = mapSensorId(sensor.getType());
      if (id == null) continue;
      sensorNames.put(sensor.getType(), id);
      sensorManager.registerListener(this, sensor, SensorManager.SENSOR_DELAY_NORMAL);
    }
    running = true;
    JSObject ret = new JSObject();
    ret.put("running", true);
    ret.put("count", sensorNames.size());
    call.resolve(ret);
  }

  @PluginMethod
  public void stop(PluginCall call) {
    stopSensors();
    JSObject ret = new JSObject();
    ret.put("running", false);
    call.resolve(ret);
  }

  private void stopSensors() {
    if (sensorManager != null && running) {
      sensorManager.unregisterListener(this);
    }
    sensorNames.clear();
    running = false;
  }

  @Override
  public void onSensorChanged(SensorEvent event) {
    String id = sensorNames.get(event.sensor.getType());
    if (id == null) return;

    List<Double> sensorValues = new ArrayList<>();
    for (float v : event.values) sensorValues.add((double) v);
    JSArray values = new JSArray(sensorValues);

    JSObject payload = new JSObject();
    payload.put("id", id);
    payload.put("androidType", event.sensor.getType());
    payload.put("name", event.sensor.getName());
    payload.put("timestamp", System.currentTimeMillis());
    payload.put("values", values);
    notifyListeners("reading", payload);
  }

  @Override
  public void onAccuracyChanged(Sensor sensor, int accuracy) {}

  private String mapSensorId(int type) {
    switch (type) {
      case Sensor.TYPE_ACCELEROMETER:
      case Sensor.TYPE_LINEAR_ACCELERATION:
      case Sensor.TYPE_GRAVITY:
      case Sensor.TYPE_GYROSCOPE:
        return "motion";
      case Sensor.TYPE_ROTATION_VECTOR:
      case Sensor.TYPE_GAME_ROTATION_VECTOR:
      case Sensor.TYPE_GEOMAGNETIC_ROTATION_VECTOR:
        return "orientation";
      case Sensor.TYPE_LIGHT:
        return "light";
      case Sensor.TYPE_MAGNETIC_FIELD:
        return "magnetometer";
      case Sensor.TYPE_PRESSURE:
        return "barometer";
      case Sensor.TYPE_PROXIMITY:
        return "proximity";
      case Sensor.TYPE_STEP_COUNTER:
      case Sensor.TYPE_STEP_DETECTOR:
        return "steps";
      case Sensor.TYPE_HEART_RATE:
        return "heart-rate";
      default:
        return "android-" + type;
    }
  }
}
