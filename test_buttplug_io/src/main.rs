use buttplug_client::{ButtplugClient};
use buttplug_core::message::InputType;
use buttplug_client_in_process::ButtplugInProcessClientConnectorBuilder;
use buttplug_server::ButtplugServerBuilder;
use buttplug_server::device::ServerDeviceManagerBuilder;
use buttplug_server_hwmgr_btleplug::BtlePlugCommunicationManagerBuilder;
use futures::StreamExt;
use std::time::Duration;
use tokio::time::sleep;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .init();

    println!("========================================");
    println!("BUTTPLUG IO CORE TEST SCRIPT");
    println!("========================================");
    
    println!("Starte Buttplug Server In-Process...");
    
    // Create device manager builder with btleplug
    let mut device_manager_builder = ServerDeviceManagerBuilder::new(
        buttplug_server_device_config::load_protocol_configs(&None, &None, false).unwrap().finish().unwrap()
    );
    device_manager_builder.comm_manager(BtlePlugCommunicationManagerBuilder::default());
    let device_manager = device_manager_builder.finish().unwrap();

    let server = ButtplugServerBuilder::new(device_manager).finish().unwrap();

    let connector = ButtplugInProcessClientConnectorBuilder::default()
        .server(server)
        .finish();

    let client = ButtplugClient::new("LELO V3 Test Client");
    println!("Verbinde Client...");
    client.connect(connector).await.unwrap();

    let mut event_stream = client.event_stream();

    println!("Starte Bluetooth-Suche nach Toys...");
    client.start_scanning().await.unwrap();

    let mut found_toy = None;

    let mut scan_timeout = Box::pin(sleep(Duration::from_secs(30)));

    loop {
        tokio::select! {
            event = event_stream.next() => {
                if let Some(buttplug_client::ButtplugClientEvent::DeviceAdded(device)) = event {
                    let name_upper = device.name().to_uppercase();
                    if name_upper.contains("F1S") || name_upper.contains("LELO") {
                        println!("✅ Gefundenes Toy: {}", device.name());
                        found_toy = Some(device);
                        let _ = client.stop_scanning().await;
                        break;
                    } else {
                        println!("Ignoriere anderes Gerät: {}", device.name());
                    }
                }
            }
            _ = &mut scan_timeout => {
                println!("❌ Scan-Timeout. Kein LELO F1S gefunden.");
                break;
            }
        }
    }

    if let Some(toy) = found_toy {
        println!("\n🚀 Gerät wurde dem Client hinzugefügt!");
        println!("(Wenn der Auth-Flow aktiv ist, hast du hier bereits erfolgreich den Knopf gedrückt!)");
        
        let mut device_events = toy.event_stream();

        if let Ok(bat) = toy.battery().await {
            println!("🔋 Batterie-Level: {}", bat); // Is u32 now
        } else {
            println!("🔋 Batterie-Level: Nicht verfügbar");
        }
        
        if let Ok(rssi) = toy.rssi().await {
            println!("📶 Signalstärke (RSSI): {}", rssi);
        }

        println!("\nLese Sensoren aktiv aus (Read)...");
        
        if toy.input_available(InputType::Pressure) {
            if let Ok(val) = toy.run_input_read(InputType::Pressure).await {
                println!("🌡️ Druck/Temperatur: {:?}", val);
            }
            println!("Abonniere Pressure...");
            let _ = toy.run_input_subscribe(InputType::Pressure).await;
        }

        if toy.input_available(InputType::Depth) {
            if let Ok(val) = toy.run_input_read(InputType::Depth).await {
                println!("📏 Depth Sensor: {:?}", val);
            }
            println!("Abonniere Depth...");
            let _ = toy.run_input_subscribe(InputType::Depth).await;
        }

        if toy.input_available(InputType::Accelerometer) {
            if let Ok(val) = toy.run_input_read(InputType::Accelerometer).await {
                println!("🚀 Accelerometer: {:?}", val);
            }
            println!("Abonniere Accelerometer...");
            let _ = toy.run_input_subscribe(InputType::Accelerometer).await;
        }

        if toy.input_available(InputType::Button) {
            if let Ok(val) = toy.run_input_read(InputType::Button).await {
                println!("🔘 Button Status: {:?}", val);
            }
            println!("Abonniere Button...");
            let _ = toy.run_input_subscribe(InputType::Button).await;
        }

        println!("\n>>> Warte 10 Sekunden auf Live-Daten von abonnierten Sensoren... <<<");
        let mut live_timeout = Box::pin(sleep(Duration::from_secs(10)));
        loop {
            tokio::select! {
                event = device_events.next() => {
                    if let Some(e) = event {
                        if let buttplug_client::ButtplugClientDeviceEvent::Message(
                            buttplug_core::message::ButtplugServerMessageV4::InputReading(reading)
                        ) = e {
                            let feat_id = reading.feature_index();
                            let val = reading.reading();
                            println!("📡 LIVE SENSOR (Feature {}): {:?}", feat_id, val);
                        }
                    }
                }
                _ = &mut live_timeout => {
                    break;
                }
            }
        }
        
        println!("\nTest erfolgreich beendet!");
    }

    client.disconnect().await.unwrap();
}
