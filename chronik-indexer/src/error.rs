use bitcoinsuite_error::{ErrorMeta, Report};

use crate::{
    broadcast::BroadcastError, BlocksError, ScriptHistoryError, SlpIndexerError, UtxosError,
};

pub fn report_to_error_meta(report: &Report) -> Option<&dyn ErrorMeta> {
    if let Some(err) = report.downcast_ref::<BlocksError>() {
        Some(err)
    } else if let Some(err) = report.downcast_ref::<BroadcastError>() {
        Some(err)
    } else if let Some(err) = report.downcast_ref::<SlpIndexerError>() {
        Some(err)
    } else if let Some(err) = report.downcast_ref::<ScriptHistoryError>() {
        Some(err)
    } else if let Some(err) = report.downcast_ref::<UtxosError>() {
        Some(err)
    } else {
        None
    }
}
