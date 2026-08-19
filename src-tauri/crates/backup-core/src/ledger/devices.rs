//! The legacy DJI transmitter pairing table.

use rusqlite::params;

use crate::device::PairedDevice;
use crate::error::CoreError;

use super::Ledger;
use super::sql::{parse_transmitter, to_i64, transmitter_name};

impl Ledger {
    /// Records the legacy DJI transmitter pairing. `paired_devices` seeds the dynamic-source
    /// migration and the connected-hardware acceptance test; nothing pairs in bulk.
    pub fn pair_device(&mut self, device: &PairedDevice, paired_at: &str) -> Result<(), CoreError> {
        self.connection
            .execute(
                r#"INSERT INTO paired_devices(
                     transmitter, volume_uuid, protocol, media_name, nominal_capacity, paired_at
                   ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                   ON CONFLICT(transmitter) DO UPDATE SET
                     volume_uuid = excluded.volume_uuid,
                     protocol = excluded.protocol,
                     media_name = excluded.media_name,
                     nominal_capacity = excluded.nominal_capacity,
                     paired_at = excluded.paired_at"#,
                params![
                    transmitter_name(device.transmitter),
                    device.expected_uuid,
                    device.expected_protocol,
                    device.expected_media_name,
                    to_i64(device.expected_capacity)?,
                    paired_at,
                ],
            )
            .map_err(CoreError::Ledger)?;
        Ok(())
    }

    pub fn paired_device_count(&self) -> Result<u64, CoreError> {
        let count = self
            .connection
            .query_row("SELECT COUNT(*) FROM paired_devices", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(CoreError::Ledger)?;
        u64::try_from(count).map_err(|_| CoreError::LedgerCorrupt)
    }

    pub fn paired_devices(&self) -> Result<Vec<PairedDevice>, CoreError> {
        let mut statement = self
            .connection
            .prepare(
                r#"SELECT transmitter, volume_uuid, protocol, media_name, nominal_capacity
                   FROM paired_devices ORDER BY transmitter"#,
            )
            .map_err(CoreError::Ledger)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })
            .map_err(CoreError::Ledger)?;
        rows.map(|row| {
            let (transmitter, expected_uuid, expected_protocol, expected_media_name, capacity) =
                row.map_err(CoreError::Ledger)?;
            Ok(PairedDevice {
                transmitter: parse_transmitter(&transmitter)?,
                expected_uuid,
                expected_protocol,
                expected_media_name,
                expected_capacity: u64::try_from(capacity).map_err(|_| CoreError::LedgerCorrupt)?,
            })
        })
        .collect()
    }
}
