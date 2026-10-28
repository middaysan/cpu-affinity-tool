//! Explicit compatible-file updates on the existing application writer.
use super::*;
use crate::app::models::CURRENT_APP_STATE_VERSION;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum SettingsUpdate {
    #[default]
    None,
    Required {
        from_version: u32,
    },
    Updating,
    Checking,
    NeedsAttention,
}

impl HeadlessTuning {
    fn checked_update_source(&self) -> Result<AppStateStorage, Box<dyn std::error::Error>> {
        if self.state.save_blocked() || self.legacy_recovery {
            return Err("Saved settings require attention before they can be updated".into());
        }
        let path = self
            .persistence
            .source_path()
            .ok_or("Saved settings location is unavailable")?;
        let saved = AppStateStorage::read_saved_for_settings_update(&path)?;
        if serde_json::to_vec(&saved)? != serde_json::to_vec(&self.state)? {
            return Err(
                "Saved settings changed outside this app. Restart before updating them.".into(),
            );
        }
        Ok(saved)
    }

    pub(super) fn detect_settings_update(&mut self) {
        self.settings_update = SettingsUpdate::None;
        if self.state.loaded_version < CURRENT_APP_STATE_VERSION {
            if let Ok(saved) = self.checked_update_source() {
                if saved.loaded_version < CURRENT_APP_STATE_VERSION {
                    self.settings_update = SettingsUpdate::Required {
                        from_version: saved.loaded_version,
                    };
                }
            }
        }
    }

    pub(crate) fn update_saved_settings(&mut self) -> Result<(), SaveError> {
        let saved = self.checked_update_source().map_err(|error| {
            self.settings_update = SettingsUpdate::None;
            self.native_failure(persistence::failure("saved settings update", error))
        })?;
        if saved.loaded_version >= CURRENT_APP_STATE_VERSION {
            self.settings_update = SettingsUpdate::None;
            return Err(SaveError::Invalid);
        }
        let required = SettingsUpdate::Required {
            from_version: saved.loaded_version,
        };
        if let Err(error) = self.publish_candidate(self.state.clone(), self.rules.clone()) {
            self.settings_update = required;
            return Err(error);
        }
        self.settings_update = SettingsUpdate::Checking;
        Ok(())
    }

    pub(crate) fn prepare_settings_check(&mut self) -> Result<(), SaveError> {
        if self.settings_update != SettingsUpdate::NeedsAttention {
            return Err(SaveError::Invalid);
        }
        self.settings_update = SettingsUpdate::Checking;
        Ok(())
    }

    pub(crate) fn check_updated_settings(&mut self) -> Result<(), SaveError> {
        if self.controller.is_durable() {
            if let Err(error) = self.verify_saved().and_then(|()| self.reconcile_manual()) {
                self.settings_update = SettingsUpdate::NeedsAttention;
                return Err(error);
            }
        } else {
            self.start_selected_recovery();
        }
        if self.native_admitted && self.operation_error.is_none() && self.controller.can_admit() {
            self.settings_update = SettingsUpdate::None;
            Ok(())
        } else {
            self.settings_update = SettingsUpdate::NeedsAttention;
            Err(SaveError::Recovering)
        }
    }
}
