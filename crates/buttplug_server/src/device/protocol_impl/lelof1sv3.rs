// Buttplug Rust Source Code File - See https://buttplug.io for more info.
//
// Copyright 2016-2026 Nonpolynomial Labs LLC. All rights reserved.
//
// Licensed under the BSD 3-Clause license. See LICENSE file in the project root
// for full license information.


use futures::{FutureExt, StreamExt};
use crate::device::{
  hardware::{
    Hardware,
    HardwareEvent,
    HardwareSubscribeCmd,
    HardwareUnsubscribeCmd,
    HardwareWriteCmd,
  },
  protocol::{
    ProtocolHandler,
    ProtocolIdentifier,
    ProtocolInitializer,
    generic_protocol_initializer_setup,
  },
};
use async_trait::async_trait;
use buttplug_core::errors::ButtplugDeviceError;
use buttplug_server_device_config::Endpoint;
use buttplug_server_device_config::{
  ProtocolCommunicationSpecifier,
  ServerDeviceDefinition,
  UserDeviceIdentifier,
};
use std::sync::Arc;
use uuid::{Uuid, uuid};

const LELO_F1S_V3_PROTOCOL_UUID: Uuid = uuid!("85c59ac5-89ee-4549-8958-ce5449226a5c");
generic_protocol_initializer_setup!(LeloF1sV3, "lelo-f1sv3");

#[derive(Default)]
pub struct LeloF1sV3Initializer {}

#[async_trait]
impl ProtocolInitializer for LeloF1sV3Initializer {
  async fn initialize(
    &mut self,
    hardware: Arc<Hardware>,
    _: &ServerDeviceDefinition,
  ) -> Result<Arc<dyn ProtocolHandler>, ButtplugDeviceError> {
    let sec_endpoint = Endpoint::Whitelist;

    let pwd_res = hardware.read_value(&crate::device::hardware::HardwareReadCmd::new(LELO_F1S_V3_PROTOCOL_UUID, sec_endpoint, 128, 500)).await?;
    let mut n = pwd_res.data().to_vec();
    
    info!("Lelo F1s V3 Auth: Initial read {} bytes: {:?}", n.len(), n);

    if !n.is_empty() && n[0] == 0x01 {
      debug!("Lelo F1s V3 is already authorised! (Found 0x01)");
      return Ok(Arc::new(LeloF1sV3::new(true)));
    }

    info!("Lelo F1s V3 waiting for auth: Tap the device's power button to complete connection.");
    println!("\n\n=======================================================");
    println!("⚠️  PLEASE TAP THE POWER BUTTON ON THE F1SV3 NOW!  ⚠️");
    println!("=======================================================\n\n");

    let mut event_receiver = hardware.event_stream();
    hardware.subscribe(&HardwareSubscribeCmd::new(LELO_F1S_V3_PROTOCOL_UUID, sec_endpoint)).await?;

    let noauth: Vec<u8> = vec![0; 8];
    let mut password = Vec::new();

    info!("Waiting for password notification... (Timeout in 30 seconds)");
    for _ in 0..300 {
        if let Ok(event) = event_receiver.try_recv() {
            if let HardwareEvent::Notification(_, _, data) = event {
                if data.len() == 8 && data != noauth {
                    info!("Lelo F1s V3 Auth: Received password via NOTIFY: {:?}", data);
                    password = data;
                    break;
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    hardware.unsubscribe(&HardwareUnsubscribeCmd::new(LELO_F1S_V3_PROTOCOL_UUID, sec_endpoint)).await?;

    if password.is_empty() {
        return Err(ButtplugDeviceError::ProtocolSpecificError(
            "LeloF1sV3".to_owned(),
            "Did not receive a valid password notification within the timeout.".to_owned(),
        ));
    }

    info!("Writing password back to device...");
    hardware
      .write_value(&HardwareWriteCmd::new(
        &[LELO_F1S_V3_PROTOCOL_UUID],
        sec_endpoint,
        password,
        true,
      ))
      .await?;

    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
    let verify_res = hardware.read_value(&crate::device::hardware::HardwareReadCmd::new(LELO_F1S_V3_PROTOCOL_UUID, sec_endpoint, 128, 500)).await?;
    let v_data = verify_res.data();

    if !v_data.is_empty() && v_data[0] == 0x01 {
      debug!("Lelo F1s V3 is authorised! Result starts with 0x01.");
      return Ok(Arc::new(LeloF1sV3::new(true)));
    } else {
      info!("Device returned {:?} instead of starting with 0x01. Handshake failed.", v_data);
      return Err(ButtplugDeviceError::ProtocolSpecificError(
        "LeloF1sV3".to_owned(),
        "Handshake failed. Result was not 01. Did you press the power button?".to_owned(),
      ));
    }
  }
}

pub struct LeloF1sV3 {
  speeds: [std::sync::atomic::AtomicU8; 2],
  write_with_response: bool,
  subscribed_sensors: Arc<std::sync::atomic::AtomicU8>,
  sensor_indices: Arc<[std::sync::atomic::AtomicU32; 4]>,
  event_stream: tokio::sync::broadcast::Sender<crate::message::ButtplugServerDeviceMessage>,
}

impl LeloF1sV3 {
  pub fn new(write_with_response: bool) -> Self {
    let (sender, _) = tokio::sync::broadcast::channel(256);
    Self {
      write_with_response,
      speeds: [std::sync::atomic::AtomicU8::new(0), std::sync::atomic::AtomicU8::new(0)],
      subscribed_sensors: Arc::new(std::sync::atomic::AtomicU8::new(0)),
      sensor_indices: Arc::new([
        std::sync::atomic::AtomicU32::new(0),
        std::sync::atomic::AtomicU32::new(0),
        std::sync::atomic::AtomicU32::new(0),
        std::sync::atomic::AtomicU32::new(0),
      ]),
      event_stream: sender,
    }
  }
}

impl ProtocolHandler for LeloF1sV3 {
  fn event_stream(
    &self,
  ) -> std::pin::Pin<Box<dyn futures::Stream<Item = crate::message::ButtplugServerDeviceMessage> + Send>> {
    buttplug_core::util::stream::convert_broadcast_receiver_to_stream(self.event_stream.subscribe()).boxed()
  }

  fn handle_output_vibrate_cmd(
    &self,
    feature_index: u32,
    feature_id: uuid::Uuid,
    speed: u32,
  ) -> Result<Vec<crate::device::hardware::HardwareCommand>, ButtplugDeviceError> {
    self.speeds[feature_index as usize].store(speed as u8, std::sync::atomic::Ordering::Relaxed);
    let mut cmd_vec = vec![0x1];
    self
      .speeds
      .iter()
      .for_each(|v| cmd_vec.push(v.load(std::sync::atomic::Ordering::Relaxed)));
    Ok(vec![
      crate::device::hardware::HardwareWriteCmd::new(
        &[feature_id],
        Endpoint::Tx,
        cmd_vec,
        self.write_with_response,
      )
      .into(),
    ])
  }

  fn handle_input_read_cmd(
    &self,
    device_index: u32,
    device: Arc<Hardware>,
    feature_index: u32,
    feature_id: uuid::Uuid,
    sensor_type: buttplug_core::message::InputType,
  ) -> futures::future::BoxFuture<'_, Result<buttplug_core::message::InputReadingV4, ButtplugDeviceError>> {
    let endpoint = match sensor_type {
      buttplug_core::message::InputType::Pressure => Endpoint::RxPressure,
      buttplug_core::message::InputType::Depth => Endpoint::RxTouch,
      buttplug_core::message::InputType::Accelerometer => Endpoint::RxAccel,
      buttplug_core::message::InputType::Button => Endpoint::Generic1,
      buttplug_core::message::InputType::Battery => {
        return crate::device::protocol::ProtocolHandler::handle_battery_level_cmd(self, device_index, device, feature_index, feature_id);
      }
      _ => return futures::future::ready(Err(ButtplugDeviceError::UnhandledCommand(
        "Unsupported sensor type".to_owned()
      ))).boxed(),
    };

    let device_clone = device.clone();
    async move {
      let result = device_clone.read_value(&crate::device::hardware::HardwareReadCmd::new(feature_id, endpoint, 128, 500)).await?;
      let data = result.data();
      let reading = match sensor_type {
        buttplug_core::message::InputType::Pressure => {
          if data.len() >= 8 && data[3] == 0xFF {
            let pressure = (data[4] as u32) << 24 | (data[5] as u32) << 16 | (data[6] as u32) << 8 | (data[7] as u32);
            buttplug_core::message::InputTypeReading::Pressure(buttplug_core::message::InputValue::new(pressure / 100))
          } else {
            return Err(ButtplugDeviceError::ProtocolSpecificError("LeloF1sV3".to_owned(), "Invalid pressure data length".to_owned()));
          }
        },
        buttplug_core::message::InputType::Depth => {
          if data.len() >= 2 {
            buttplug_core::message::InputTypeReading::Depth(buttplug_core::message::InputValue::new(data[1]))
          } else {
            return Err(ButtplugDeviceError::ProtocolSpecificError("LeloF1sV3".to_owned(), "Invalid depth data length".to_owned()));
          }
        },
        buttplug_core::message::InputType::Accelerometer => {
          if data.len() >= 6 {
            let x = ((data[0] as i16) << 8 | data[1] as i16) as i32;
            let y = ((data[2] as i16) << 8 | data[3] as i16) as i32;
            let z = ((data[4] as i16) << 8 | data[5] as i16) as i32;
            buttplug_core::message::InputTypeReading::Accelerometer(buttplug_core::message::InputValue::new([x, y, z]))
          } else {
            return Err(ButtplugDeviceError::ProtocolSpecificError("LeloF1sV3".to_owned(), "Invalid accel data length".to_owned()));
          }
        },
        buttplug_core::message::InputType::Button => {
          if data.len() >= 1 {
            buttplug_core::message::InputTypeReading::Button(buttplug_core::message::InputValue::new(data[0]))
          } else {
            return Err(ButtplugDeviceError::ProtocolSpecificError("LeloF1sV3".to_owned(), "Invalid button data length".to_owned()));
          }
        },
        _ => unreachable!(),
      };
      Ok(buttplug_core::message::InputReadingV4::new(device_index, feature_index, reading))
    }.boxed()
  }

  fn handle_input_subscribe_cmd(
    &self,
    device_index: u32,
    device: Arc<Hardware>,
    feature_index: u32,
    feature_id: uuid::Uuid,
    sensor_type: buttplug_core::message::InputType,
  ) -> futures::future::BoxFuture<'_, Result<(), ButtplugDeviceError>> {
    let (endpoint, idx) = match sensor_type {
      buttplug_core::message::InputType::Pressure => (Endpoint::RxPressure, 0),
      buttplug_core::message::InputType::Depth => (Endpoint::RxTouch, 1),
      buttplug_core::message::InputType::Accelerometer => (Endpoint::RxAccel, 2),
      buttplug_core::message::InputType::Button => (Endpoint::Generic1, 3),
      _ => return futures::future::ready(Err(ButtplugDeviceError::UnhandledCommand(
        "Unsupported sensor type".to_owned()
      ))).boxed(),
    };

    self.sensor_indices[idx].store(feature_index, std::sync::atomic::Ordering::Relaxed);

    let stream_sensors = self.subscribed_sensors.clone();
    let sensor_indices = self.sensor_indices.clone();
    let sender = self.event_stream.clone();
    let mut hardware_stream = device.event_stream();

    async move {
      let sensors = stream_sensors.load(std::sync::atomic::Ordering::Relaxed);
      if sensors == 0 {
        buttplug_core::spawn!("Lelo F1s V3 subscription event handler", async move {
          while let Ok(info) = hardware_stream.recv().await {
            if sender.receiver_count() == 0 {
              return;
            }
            if let HardwareEvent::Notification(_, ep, data) = info {
              match ep {
                Endpoint::RxPressure => { // Pressure
                  if data.len() >= 8 && data[3] == 0xFF {
                    let pressure = (data[4] as u32) << 24 | (data[5] as u32) << 16 | (data[6] as u32) << 8 | (data[7] as u32);
                    let feat_idx = sensor_indices[0].load(std::sync::atomic::Ordering::Relaxed);
                    let _ = sender.send(buttplug_core::message::InputReadingV4::new(device_index, feat_idx, buttplug_core::message::InputTypeReading::Pressure(buttplug_core::message::InputValue::new(pressure / 100))).into());
                  }
                },
                Endpoint::RxTouch => { // Depth
                  if data.len() >= 2 {
                    let feat_idx = sensor_indices[1].load(std::sync::atomic::Ordering::Relaxed);
                    let _ = sender.send(buttplug_core::message::InputReadingV4::new(device_index, feat_idx, buttplug_core::message::InputTypeReading::Depth(buttplug_core::message::InputValue::new(data[1]))).into());
                  }
                },
                Endpoint::RxAccel => { // Accelerometer
                  if data.len() >= 6 {
                    let x = ((data[0] as i16) << 8 | data[1] as i16) as i32;
                    let y = ((data[2] as i16) << 8 | data[3] as i16) as i32;
                    let z = ((data[4] as i16) << 8 | data[5] as i16) as i32;
                    let feat_idx = sensor_indices[2].load(std::sync::atomic::Ordering::Relaxed);
                    let _ = sender.send(buttplug_core::message::InputReadingV4::new(device_index, feat_idx, buttplug_core::message::InputTypeReading::Accelerometer(buttplug_core::message::InputValue::new([x, y, z]))).into());
                  }
                },
                Endpoint::Generic1 => { // Button
                  if data.len() >= 1 {
                    let feat_idx = sensor_indices[3].load(std::sync::atomic::Ordering::Relaxed);
                    let _ = sender.send(buttplug_core::message::InputReadingV4::new(device_index, feat_idx, buttplug_core::message::InputTypeReading::Button(buttplug_core::message::InputValue::new(data[0]))).into());
                  }
                },
                _ => {}
              }
            }
          }
        });
      }

      device
        .subscribe(&HardwareSubscribeCmd::new(feature_id, endpoint))
        .await?;

      stream_sensors.store(
        stream_sensors.load(std::sync::atomic::Ordering::Relaxed) | (1 << feature_index),
        std::sync::atomic::Ordering::Relaxed,
      );
      Ok(())
    }
    .boxed()
  }

  fn handle_input_unsubscribe_cmd(
    &self,
    device: Arc<Hardware>,
    feature_index: u32,
    feature_id: uuid::Uuid,
    sensor_type: buttplug_core::message::InputType,
  ) -> futures::future::BoxFuture<'_, Result<(), ButtplugDeviceError>> {
    let endpoint = match sensor_type {
      buttplug_core::message::InputType::Pressure => Endpoint::RxPressure,
      buttplug_core::message::InputType::Depth => Endpoint::RxTouch,
      buttplug_core::message::InputType::Accelerometer => Endpoint::RxAccel,
      buttplug_core::message::InputType::Button => Endpoint::Generic1,
      _ => return futures::future::ready(Ok(())).boxed(),
    };
    
    let sensors = self.subscribed_sensors.clone();
    async move {
      sensors.store(
        sensors.load(std::sync::atomic::Ordering::Relaxed) & !(1 << feature_index),
        std::sync::atomic::Ordering::Relaxed,
      );
      
      device
        .unsubscribe(&crate::device::hardware::HardwareUnsubscribeCmd::new(
          feature_id,
          endpoint,
        ))
        .await?;
      Ok(())
    }
    .boxed()
  }
}
