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

    // The Lelo F1s V3 has a very specific pairing flow:
    // * First the device is turned on in BLE mode (long press)
    // * Then the security endpoint (Whitelist) needs to be read (which we can do via subscribe)
    // * If it returns 0x00,00,00,00,00,00,00,00 the connection isn't not authorised
    // * To authorize, the password must be writen to the characteristic.
    // * If the password is unknown (buttplug lacks a storage mechanism right now), the power button
    //   must be pressed to send the password
    // * The password must not be sent whilst subscribed to the endpoint
    // * Once the password has been sent, the endpoint can be read for status again
    // * If it returns 0x00,00,00,00,00,00,00,00 the connection is authorised
    let mut event_receiver = hardware.event_stream();
    hardware
      .subscribe(&HardwareSubscribeCmd::new(
        LELO_F1S_V3_PROTOCOL_UUID,
        sec_endpoint,
      ))
      .await?;
    let noauth: Vec<u8> = vec![0; 8];
    let authed: Vec<u8> = vec![1, 0, 0, 0, 0, 0, 0, 0];

    info!("Lelo F1s V3 waiting for auth: Tap the device's power button to complete connection.");
    println!("\n\n=======================================================");
    println!("⚠️  PLEASE TAP THE POWER BUTTON ON THE F1SV3 NOW!  ⚠️");
    println!("=======================================================\n\n");

    loop {
      let event = event_receiver.recv().await;
      if let Ok(HardwareEvent::Notification(_, _, n)) = event {
        if n.eq(&noauth) {
          info!("Lelo F1s V3 explicitly reported not authorised.");
        } else if n.eq(&authed) {
          debug!("Lelo F1s V3 is authorised!");
          return Ok(Arc::new(LeloF1sV3::new(true)));
        } else {
          debug!("Lelo F1s V3 gave us a password: {:?}", n);
          // Can't send whilst subscribed
          hardware
            .unsubscribe(&HardwareUnsubscribeCmd::new(
              LELO_F1S_V3_PROTOCOL_UUID,
              sec_endpoint,
            ))
            .await?;
          // Send with response
          hardware
            .write_value(&HardwareWriteCmd::new(
              &[LELO_F1S_V3_PROTOCOL_UUID],
              sec_endpoint,
              n,
              true,
            ))
            .await?;
          // Get back to the loop
          hardware
            .subscribe(&HardwareSubscribeCmd::new(
              LELO_F1S_V3_PROTOCOL_UUID,
              sec_endpoint,
            ))
            .await?;
        }
      } else {
        return Err(ButtplugDeviceError::ProtocolSpecificError(
          "LeloF1sV3".to_owned(),
          "Lelo F1s V3 didn't provided valid security handshake".to_owned(),
        ));
      }
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
