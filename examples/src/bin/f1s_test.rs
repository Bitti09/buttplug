use buttplug_client::{ButtplugClient, ButtplugClientEvent, ButtplugClientDevice, ButtplugClientDeviceEvent};
use buttplug_core::message::{InputType, ButtplugServerMessageV4};
use buttplug_client_in_process::ButtplugInProcessClientConnectorBuilder;
use buttplug_server::ButtplugServerBuilder;
use buttplug_server::device::ServerDeviceManagerBuilder;
use buttplug_server_device_config::load_protocol_configs;
use buttplug_server_hwmgr_btleplug::BtlePlugCommunicationManagerBuilder;
use futures::StreamExt;
use tokio::time::{sleep, Duration};

async fn setup_device(device: ButtplugClientDevice) {
  println!("+++ Device Connected: {} +++", device.name());

  let mut device_stream = device.event_stream();

  let sensors = [
    InputType::Pressure,
    InputType::Depth,
    InputType::Accelerometer,
  ];

  for sensor in sensors.iter() {
    if device.input_available(*sensor) {
      println!("Subscribing to {:?} sensor...", sensor);
      if let Err(e) = device.run_input_subscribe(*sensor).await {
        println!("Error subscribing to {:?}: {:?}", sensor, e);
      } else {
        println!("Successfully subscribed to {:?}", sensor);
      }
    } else {
      println!("Device does not support {:?} sensor", sensor);
    }
  }

  if device.input_available(InputType::Battery) {
    println!("Reading Battery sensor...");
    match device.run_input_read(InputType::Battery).await {
      Ok(reading) => println!("Got Battery Reading: {:?}", reading),
      Err(e) => println!("Error reading Battery: {:?}", e),
    }
  }
  tokio::spawn(async move {
    let mut got_pressure = false;
    let mut got_depth = false;
    let mut got_accel = false;

    while let Some(event) = device_stream.next().await {
      if let ButtplugClientDeviceEvent::Message(ButtplugServerMessageV4::InputReading(reading)) = event {
        println!("Got Input Reading: {:?}", reading);
        let r_str = format!("{:?}", reading);
        if r_str.contains("Pressure") { got_pressure = true; }
        if r_str.contains("Depth") { got_depth = true; }
        if r_str.contains("Accelerometer") { got_accel = true; }

        if got_pressure && got_depth && got_accel {
          println!("\n*** ALL SENSORS READ SUCCESSFULLY! TESTING MOTORS. ***\n");
          for (i, feature) in device.outputs(buttplug_core::message::OutputType::Vibrate).iter().enumerate() {
            println!("Testing Vibrator {} (Feature Index {}) at 25%...", i, feature.feature_index());
            if let Err(e) = feature.run_output(&buttplug_client::device::ClientDeviceOutputCommand::Vibrate(0.25.into())).await {
              println!("Error vibrating: {:?}", e);
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            let _ = feature.run_output(&buttplug_client::device::ClientDeviceOutputCommand::Vibrate(0.0.into())).await;
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
          }
          std::process::exit(0);
        }
      } else {
        println!("Got Event: {:?}", event);
      }
    }
  });
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
  tracing_subscriber::fmt()
    .with_max_level(tracing::Level::DEBUG)
    .init();

  let dc: Option<String> = None;
  let uc: Option<String> = None;

  let dcm = load_protocol_configs(&dc, &uc, false)
    .unwrap()
    .finish()
    .unwrap();

  let mut server_builder = ServerDeviceManagerBuilder::new(dcm);
  server_builder.comm_manager(BtlePlugCommunicationManagerBuilder::default());

  let sb = ButtplugServerBuilder::new(server_builder.finish().unwrap());
  let server = sb.finish().unwrap();
  let connector = ButtplugInProcessClientConnectorBuilder::default()
    .server(server)
    .finish();
  let client = ButtplugClient::new("Test Client");
  client.connect(connector).await.unwrap();

  let mut event_stream = client.event_stream();

  println!("Starting device scan...");
  if let Err(e) = client.start_scanning().await {
    println!("Error starting scan: {}", e);
  }

  println!("Scanning and waiting for events for 60 seconds...");
  
  let sleep_fut = sleep(Duration::from_secs(60));
  tokio::pin!(sleep_fut);

  loop {
    tokio::select! {
      _ = &mut sleep_fut => {
        println!("60 seconds elapsed. Exiting.");
        break;
      }
      event = event_stream.next() => {
        if let Some(event) = event {
          match event {
            ButtplugClientEvent::DeviceAdded(device) => {
              tokio::spawn(setup_device(device));
            }
            ButtplugClientEvent::DeviceRemoved(device) => {
              println!("--- Device Removed: {} ---", device.name());
            }
            _ => {}
          }
        } else {
          break;
        }
      }
    }
  }

  println!("Test complete.");
}
