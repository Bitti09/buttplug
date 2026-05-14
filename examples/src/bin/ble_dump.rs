use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::Manager;
use std::time::Duration;
use tokio::time;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manager = Manager::new().await?;
    let adapters = manager.adapters().await?;
    let central = adapters.into_iter().next().expect("No Bluetooth adapters found");

    println!("Starting scan...");
    central.start_scan(ScanFilter::default()).await?;
    time::sleep(Duration::from_secs(5)).await;

    for p in central.peripherals().await? {
        if let Some(props) = p.properties().await? {
            if let Some(name) = props.local_name {
                if name.contains("F1SV3") {
                    println!("Found F1SV3! Connecting...");
                    p.connect().await?;
                    println!("Connected! Discovering services...");
                    p.discover_services().await?;
                    for service in p.services() {
                        println!("Service: {}", service.uuid);
                        for char in service.characteristics {
                            println!("  Characteristic: {}", char.uuid);
                        }
                    }
                    p.disconnect().await?;
                    return Ok(());
                }
            }
        }
    }
    println!("F1SV3 not found!");
    Ok(())
}
