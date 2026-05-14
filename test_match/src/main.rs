use btleplug::api::{Central, Characteristic, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::Manager;
use std::time::Duration;
use tokio::time;

const UUID_MOTOR_CONTROL: &str = "0000fff1-0000-1000-8000-00805f9b34fb";
const UUID_CRUISE_CONTROL: &str = "00000aa5-0000-1000-8000-00805f9b34fb";
const UUID_VIBRATOR_SETTING: &str = "00000a0d-0000-1000-8000-00805f9b34fb";
const UUID_KEY_STATE: &str = "00000a0f-0000-1000-8000-00805f9b34fb";
const UUID_WAKE_UP: &str = "00000aa1-0000-1000-8000-00805f9b34fb";
const UUID_HALL_SENSOR: &str = "00000aa3-0000-1000-8000-00805f9b34fb";
const UUID_DEPTH_SENSOR: &str = "00000a0b-0000-1000-8000-00805f9b34fb";
const UUID_ACCELEROMETER: &str = "00000a0c-0000-1000-8000-00805f9b34fb";
const UUID_PRESSURE_TEMP: &str = "00000a0a-0000-1000-8000-00805f9b34fb";
const UUID_BUTTONS: &str = "00000aa4-0000-1000-8000-00805f9b34fb";
const UUID_USE_LOG: &str = "00000a04-0000-1000-8000-00805f9b34fb";
const UUID_BATTERY: &str = "00002a19-0000-1000-8000-00805f9b34fb";
const UUID_FIRMWARE: &str = "00002a26-0000-1000-8000-00805f9b34fb";
const UUID_SOFTWARE: &str = "00002a28-0000-1000-8000-00805f9b34fb";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manager = Manager::new().await?;
    let adapters = manager.adapters().await?;
    let central = adapters.into_iter().nth(0).expect("No Bluetooth adapters found");

    println!("Starte Bluetooth-Suche nach LELO F1S V3 (suche bis zu 15 Sekunden)...");
    central.start_scan(ScanFilter::default()).await?;

    let mut f1s_device = None;
    for _ in 0..15 {
        time::sleep(Duration::from_secs(1)).await;
        let peripherals = central.peripherals().await?;
        for peripheral in peripherals {
            if let Some(properties) = peripheral.properties().await? {
                if let Some(name) = properties.local_name {
                    if name.contains("F1S") || name.contains("F1s") {
                        println!("Gefunden: {}", name);
                        f1s_device = Some(peripheral);
                        break;
                    }
                }
            }
        }
        if f1s_device.is_some() {
            break;
        }
    }

    let device = f1s_device.expect("Konnte das F1S V3 nicht finden! Ist es an und Intiface wirklich geschlossen?");
    println!("Verbinde mit Gerät...");
    device.connect().await?;
    
    device.discover_services().await?;
    let chars = device.characteristics();

    println!("\n!!! WICHTIG !!!");
    println!("Bitte drücke JETZT einmal den Power-Knopf am Gerät, um die Verbindung zu bestätigen!");
    time::sleep(Duration::from_secs(5)).await;

    println!("\n--- LELO F1S V3 - GEFUNDENE CHARACTERISTICS ---");
    for c in chars.iter() {
        println!("UUID: {} | Properties: {:?}", c.uuid, c.properties);
    }

    println!("\n--- LELO F1S V3 - SENSOR WERTE (15 Sekunden Live-Feed) ---");

    println!("\n--- LELO F1S V3 - SENSOR WERTE (Finale Analyse) ---");

    // LELO Security Handshake Test
    let mut security_char = None;
    for c in chars.iter() {
        if c.uuid.to_string() == "00000a10-0000-1000-8000-00805f9b34fb" {
            security_char = Some(c.clone());
            break;
        }
    }

    if let Some(sec_char) = security_char {
        println!("\n[SECURITY HANDSHAKE GESTARTET]");
        if let Ok(data) = device.read(&sec_char).await {
            let hex_str: Vec<String> = data.iter().map(|b| format!("{:02X}", b)).collect();
            println!("Gelesenes Passwort: [{}]", hex_str.join(" "));
            
            if data.len() == 8 && data[0] != 0x01 {
                println!(">> BITTE JETZT SOFORT DEN POWER-BUTTON AM TOY DRÜCKEN! <<");
                for i in (1..=6).rev() {
                    print!("Warte {} Sekunden... \r", i);
                    use std::io::Write;
                    std::io::stdout().flush().unwrap();
                    time::sleep(Duration::from_secs(1)).await;
                }
                println!("\nSchreibe Passwort zurück ans Toy...");
                
                let write_res = device.write(&sec_char, &data, btleplug::api::WriteType::WithResponse).await;
                if write_res.is_err() {
                    println!("Fehler beim Schreiben des Passworts.");
                }
                
                time::sleep(Duration::from_secs(1)).await;
                if let Ok(new_data) = device.read(&sec_char).await {
                    let new_hex: Vec<String> = new_data.iter().map(|b| format!("{:02X}", b)).collect();
                    println!("Neuer Security-Status: [{}]", new_hex.join(" "));
                    
                    if new_data.len() >= 1 && new_data[0] == 0x01 {
                        println!("✅ HANDSHAKE ERFOLGREICH! Gerät ist vollständig entsperrt.");
                    } else {
                        println!("❌ Handshake fehlgeschlagen! Status ist nicht 01.");
                    }
                }
            } else if data.len() >= 1 && data[0] == 0x01 {
                println!("✅ Gerät ist bereits entsperrt (Status 0x01).");
            } else {
                println!("Unerwartetes Passwort-Format.");
            }
        }
        println!("--------------------------------------------------\n");
    }

    for c in chars.iter() {
        let uuid_str = c.uuid.to_string();
        
        let (name, parse_logic): (&str, Box<dyn Fn(&[u8]) -> String>) = match uuid_str.as_str() {
            // LELO Spezifisch
            UUID_MOTOR_CONTROL => ("Motor Control", Box::new(|d| {
                if d.len() >= 3 { format!("{:02X} {:02X} {:02X} (Main: {}%, Vib: {}%)", d[0], d[1], d[2], d[1], d[2]) } else { format!("Raw: {:?}", d) }
            })),
            UUID_CRUISE_CONTROL => ("Cruise Control", Box::new(|d| format!("Raw: {:?}", d))), 
            UUID_VIBRATOR_SETTING => ("Vibrator Setting", Box::new(|d| {
                if d.len() == 1 { format!("{} (Single Byte - Global Speed?)", d[0]) } else { format!("Raw: {:?}", d) }
            })),
            UUID_KEY_STATE => ("Key State", Box::new(|d| format!("Raw: {:?}", d))),
            UUID_WAKE_UP => ("Wake Up Mode", Box::new(|d| format!("Raw: {:?}", d))),
            UUID_HALL_SENSOR => ("Hall Sensor (Speed)", Box::new(|d| format!("Raw: {:?}", d))),
            UUID_DEPTH_SENSOR => ("Depth Sensor", Box::new(|d| {
                if d.len() >= 2 { format!("{} (0-8)", d[1]) } else { format!("Raw: {:?}", d) }
            })),
            UUID_ACCELEROMETER => ("Accelerometer", Box::new(|d| {
                if d.len() >= 7 {
                    let x = ((d[0] as i16) << 8 | d[1] as i16) as i32;
                    let y = ((d[2] as i16) << 8 | d[3] as i16) as i32;
                    let z = ((d[4] as i16) << 8 | d[5] as i16) as i32;
                    format!("[X: {}, Y: {}, Z: {}, Byte7 (Orientierung): {}]", x, y, z, d[6])
                } else {
                    format!("Raw: {:?}", d)
                }
            })),
            UUID_PRESSURE_TEMP => ("Pressure & Temp", Box::new(|d| {
                if d.len() >= 8 {
                    let temp = ((d[0] as u32) << 16 | (d[1] as u32) << 8 | (d[2] as u32)) as f32 / 100.0;
                    let pressure = ((d[4] as u32) << 24 | (d[5] as u32) << 16 | (d[6] as u32) << 8 | (d[7] as u32)) as f32 / 100.0;
                    format!("Temperatur: {:.2} °C, Druck: {:.2} mbar", temp, pressure)
                } else {
                    format!("Raw: {:?}", d)
                }
            })),
            UUID_BUTTONS => ("Buttons", Box::new(|d| {
                if d.len() >= 1 { format!("{} (V3 Button Code)", d[0]) } else { format!("Raw: {:?}", d) }
            })),
            UUID_USE_LOG => ("Use Log", Box::new(|d| {
                if d.len() == 15 { format!("15-Byte Log: {:02X?}", d) } else { format!("Raw: {:?}", d) }
            })),
            
            // Unsere "Geister"-UUIDs
            "00000a00-0000-1000-8000-00805f9b34fb" => ("Battery Voltage (mV)", Box::new(|d| {
                if d.len() >= 2 { format!("{} mV", (d[0] as u16) << 8 | (d[1] as u16)) } else { format!("Raw: {:?}", d) }
            })),
            "00000a05-0000-1000-8000-00805f9b34fb" => ("Serial Number", Box::new(|d| String::from_utf8_lossy(d).into_owned())),
            "00000a06-0000-1000-8000-00805f9b34fb" => ("MAC Address", Box::new(|d| {
                let hex_str: Vec<String> = d.iter().map(|b| format!("{:02X}", b)).collect();
                hex_str.join(":")
            })),
            "00000a07-0000-1000-8000-00805f9b34fb" => ("Password Seed (Hash)", Box::new(|d| {
                let hex_str: Vec<String> = d.iter().map(|b| format!("{:02X}", b)).collect();
                format!("[{}]", hex_str.join(" "))
            })),
            "00000a08-0000-1000-8000-00805f9b34fb" => ("Unknown Reset", Box::new(|_d| "Nur Write".to_string())),
            "00000a10-0000-1000-8000-00805f9b34fb" => ("Security Access", Box::new(|d| {
                let hex_str: Vec<String> = d.iter().map(|b| format!("{:02X}", b)).collect();
                format!("[{}]", hex_str.join(" "))
            })),
            "00000a11-0000-1000-8000-00805f9b34fb" => ("Security Override", Box::new(|d| format!("{:?}", d))),
            "00000a1a-0000-1000-8000-00805f9b34fb" => ("Advanced Motor Control", Box::new(|d| {
                let hex_str: Vec<String> = d.iter().map(|b| format!("{:02X}", b)).collect();
                format!("[{}]", hex_str.join(" "))
            })),
            "0000fff2-0000-1000-8000-00805f9b34fb" => ("Motor Statistics Log", Box::new(|d| {
                let hex_str: Vec<String> = d.iter().map(|b| format!("{:02X}", b)).collect();
                format!("[{}]", hex_str.join(" "))
            })),

            // Standard BLE 2A... UUIDs
            UUID_BATTERY => ("Battery Level (%)", Box::new(|d| {
                if d.len() >= 1 { format!("{}%", d[0]) } else { format!("Raw: {:?}", d) }
            })),
            UUID_FIRMWARE => ("Firmware Revision", Box::new(|d| String::from_utf8_lossy(d).into_owned())),
            UUID_SOFTWARE => ("Software Revision", Box::new(|d| String::from_utf8_lossy(d).into_owned())),
            "00002a00-0000-1000-8000-00805f9b34fb" => ("Device Name", Box::new(|d| String::from_utf8_lossy(d).into_owned())),
            "00002a01-0000-1000-8000-00805f9b34fb" => ("Appearance", Box::new(|d| format!("{:?}", d))),
            "00002a04-0000-1000-8000-00805f9b34fb" => ("Connection Parameters", Box::new(|d| {
                let hex_str: Vec<String> = d.iter().map(|b| format!("{:02X}", b)).collect();
                format!("[{}]", hex_str.join(" "))
            })),
            "00002a23-0000-1000-8000-00805f9b34fb" => ("System ID", Box::new(|d| {
                let hex_str: Vec<String> = d.iter().map(|b| format!("{:02X}", b)).collect();
                format!("[{}]", hex_str.join(" "))
            })),
            "00002a24-0000-1000-8000-00805f9b34fb" => ("Model Number", Box::new(|d| String::from_utf8_lossy(d).into_owned())),
            "00002a25-0000-1000-8000-00805f9b34fb" => ("Serial Number String", Box::new(|d| String::from_utf8_lossy(d).into_owned())),
            "00002a27-0000-1000-8000-00805f9b34fb" => ("Hardware Revision", Box::new(|d| String::from_utf8_lossy(d).into_owned())),
            "00002a29-0000-1000-8000-00805f9b34fb" => ("Manufacturer Name", Box::new(|d| String::from_utf8_lossy(d).into_owned())),
            "00002a2a-0000-1000-8000-00805f9b34fb" => ("Regulatory Certification", Box::new(|d| String::from_utf8_lossy(d).into_owned())),
            "00002a50-0000-1000-8000-00805f9b34fb" => ("PnP ID", Box::new(|d| {
                let hex_str: Vec<String> = d.iter().map(|b| format!("{:02X}", b)).collect();
                format!("[{}]", hex_str.join(" "))
            })),

            _ => ("Unknown", Box::new(|d| {
                let hex_str: Vec<String> = d.iter().map(|b| format!("{:02X}", b)).collect();
                format!("Raw Hex: [{}]", hex_str.join(" "))
            })),
        };

        if c.properties.contains(btleplug::api::CharPropFlags::READ) {
            // Den OAD Updater lassen wir weg, da er einen Fehler wirft
            if uuid_str.starts_with("f000ff") && uuid_str != "f000ffc1-0451-4000-b000-000000000000" { continue; }

            match device.read(&c).await {
                Ok(data) => {
                    if !data.is_empty() {
                        println!("{} ({:<25}): {}", c.uuid, name, parse_logic(&data));
                    }
                },
                Err(_) => {},
            }
        }
    }

    println!("----------------------------------");
    println!("Trenne Verbindung...");
    device.disconnect().await?;

    Ok(())
}
