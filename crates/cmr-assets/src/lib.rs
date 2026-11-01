//! Verification and import of user-owned game data.

mod aux_control_import;
mod aux_control_profile;
mod aux_control_store;
mod car_selector_import;
mod car_selector_profile;
mod car_selector_store;
mod catalog;
mod compiled_initializer_import;
mod compiled_initializer_profile;
mod compiled_initializer_store;
mod engine_audio_tables_import;
mod engine_audio_tables_profile;
mod engine_audio_tables_store;
mod hd_pack;
mod import;
mod path;
mod profile;
mod resolver;
mod rival_records_import;
mod rival_records_profile;
mod rival_records_store;
mod schema;
mod stage_weather_tables_import;
mod stage_weather_tables_profile;
mod stage_weather_tables_store;
mod static_tables_import;
mod static_tables_profile;
mod static_tables_store;
mod store;
mod surface_audio_tables_import;
mod surface_audio_tables_profile;
mod surface_audio_tables_store;
mod surface_import;
mod surface_profile;
mod surface_store;
mod tyre_tables_import;
mod tyre_tables_profile;
mod tyre_tables_store;
mod verify;

pub use aux_control_import::{
    AUX_CONTROL_DATA_FILE, AUX_CONTROL_IMPORT_SCHEMA_VERSION, AUX_CONTROL_STATE_FILE,
    AuxControlImportError, AuxControlImportFile, AuxControlImportShape, AuxControlImportSummary,
    extract_aux_control_constants,
};
pub use aux_control_profile::{
    AUX_CONTROL_PROFILE_SCHEMA_VERSION, AuxControlEncoding, AuxControlExecutable,
    AuxControlProfile, AuxControlProfileError, AuxControlSpec, ReviewedAuxControlProfile,
    ReviewedAuxControlProfileError,
};
pub use aux_control_store::{AuxControlConstantsStore, AuxControlStoreError};
pub use car_selector_import::{
    CAR_SELECTOR_DATA_FILE, CAR_SELECTOR_IMPORT_SCHEMA_VERSION, CAR_SELECTOR_STATE_FILE,
    CarSelectorImportError, CarSelectorImportFile, CarSelectorImportShape,
    CarSelectorImportSummary, extract_car_selectors,
};
pub use car_selector_profile::{
    CAR_SELECTOR_PROFILE_SCHEMA_VERSION, CarSelectorEncoding, CarSelectorExecutable,
    CarSelectorProfile, CarSelectorProfileError, CarSelectorSpec, ReviewedCarSelectorProfile,
    ReviewedCarSelectorProfileError,
};
pub use car_selector_store::{CarSelectorStore, CarSelectorStoreError};
pub use catalog::catalog_standard_source;
pub use compiled_initializer_import::{
    COMPILED_INITIALIZER_DATA_FILE, COMPILED_INITIALIZER_IMPORT_SCHEMA_VERSION,
    COMPILED_INITIALIZER_STATE_FILE, CompiledInitializerImportError, CompiledInitializerImportFile,
    CompiledInitializerImportShape, CompiledInitializerImportSummary,
    extract_compiled_car_initializer,
};
pub use compiled_initializer_profile::{
    COMPILED_INITIALIZER_PROFILE_SCHEMA_VERSION, CompiledInitializerEncoding,
    CompiledInitializerExecutable, CompiledInitializerProfile, CompiledInitializerProfileError,
    CompiledInitializerSpec, ReviewedCompiledInitializerProfile,
    ReviewedCompiledInitializerProfileError,
};
pub use compiled_initializer_store::{CompiledInitializerStore, CompiledInitializerStoreError};
pub use engine_audio_tables_import::{
    ENGINE_AUDIO_TABLES_DATA_FILE, ENGINE_AUDIO_TABLES_IMPORT_SCHEMA_VERSION,
    ENGINE_AUDIO_TABLES_STATE_FILE, EngineAudioTablesImportError, EngineAudioTablesImportFile,
    EngineAudioTablesImportShape, EngineAudioTablesImportSummary, extract_engine_audio_tables,
};
pub use engine_audio_tables_profile::{
    ENGINE_AUDIO_TABLES_PROFILE_SCHEMA_VERSION, EngineAudioTablesEncoding,
    EngineAudioTablesExecutable, EngineAudioTablesProfile, EngineAudioTablesProfileError,
    EngineAudioTablesSpec, EngineSoundBandSpec, ReviewedEngineAudioTablesProfile,
    ReviewedEngineAudioTablesProfileError,
};
pub use engine_audio_tables_store::{EngineAudioTablesStore, EngineAudioTablesStoreError};
pub use hd_pack::{HdPack, HdPackError, HdScale};
pub use import::{ImportError, ImportSummary, import_source};
pub use path::normalize_relative_path;
pub use profile::{
    AssetFile, AssetProfile, ProfileError, ReviewedProfile, ReviewedProfileError, StorageEncoding,
};
pub use resolver::{AssetIndex, ResolveError};
pub use rival_records_import::{
    RIVAL_RECORDS_DATA_FILE, RIVAL_RECORDS_IMPORT_SCHEMA_VERSION, RIVAL_RECORDS_STATE_FILE,
    RIVAL_RECORDS_STORE_DIRECTORY, RivalRecordsImportError, RivalRecordsImportFile,
    RivalRecordsImportShape, RivalRecordsImportSummary, extract_rival_records,
};
pub use rival_records_profile::{
    RIVAL_RECORDS_PROFILE_SCHEMA_VERSION, ReviewedRivalRecordsProfile,
    ReviewedRivalRecordsProfileError, RivalRecordsEncoding, RivalRecordsExecutable,
    RivalRecordsProfile, RivalRecordsProfileError, RivalRecordsSpec,
};
pub use rival_records_store::{RivalRecordsStore, RivalRecordsStoreError};
pub use schema::{
    IMPORT_STATE_SCHEMA_VERSION, PROFILE_SCHEMA_VERSION, SchemaStatus, import_state_schema_status,
    profile_schema_status,
};
pub use stage_weather_tables_import::{
    STAGE_WEATHER_TABLES_DATA_FILE, STAGE_WEATHER_TABLES_IMPORT_SCHEMA_VERSION,
    STAGE_WEATHER_TABLES_STATE_FILE, STAGE_WEATHER_TABLES_STORE_DIRECTORY,
    StageWeatherTablesImportError, StageWeatherTablesImportFile, StageWeatherTablesImportShape,
    StageWeatherTablesImportSummary, extract_stage_weather_tables,
};
pub use stage_weather_tables_profile::{
    ReviewedStageWeatherTablesProfile, ReviewedStageWeatherTablesProfileError,
    STAGE_WEATHER_TABLES_PROFILE_SCHEMA_VERSION, StageWeatherTablesEncoding,
    StageWeatherTablesExecutable, StageWeatherTablesProfile, StageWeatherTablesProfileError,
    StageWeatherTablesSpec,
};
pub use stage_weather_tables_store::{StageWeatherTablesStore, StageWeatherTablesStoreError};
pub use static_tables_import::{
    STATIC_TABLES_DATA_FILE, STATIC_TABLES_IMPORT_SCHEMA_VERSION, STATIC_TABLES_STATE_FILE,
    STATIC_TABLES_STORE_DIRECTORY, StaticTablesImportError, StaticTablesImportFile,
    StaticTablesImportShape, StaticTablesImportSummary, extract_static_tables,
};
pub use static_tables_profile::{
    ReviewedStaticTablesProfile, ReviewedStaticTablesProfileError,
    STATIC_TABLES_PROFILE_SCHEMA_VERSION, StaticTablesEncoding, StaticTablesEntryKind,
    StaticTablesEntrySpec, StaticTablesExecutable, StaticTablesProfile, StaticTablesProfileError,
    StaticTablesSpec,
};
pub use static_tables_store::{StaticTablesStore, StaticTablesStoreError};
pub use store::{AssetStore, AssetStoreError};
pub use surface_audio_tables_import::{
    SURFACE_AUDIO_TABLES_DATA_FILE, SURFACE_AUDIO_TABLES_IMPORT_SCHEMA_VERSION,
    SURFACE_AUDIO_TABLES_STATE_FILE, SurfaceAudioTablesImportError, SurfaceAudioTablesImportFile,
    SurfaceAudioTablesImportShape, SurfaceAudioTablesImportSummary, extract_surface_audio_tables,
};
pub use surface_audio_tables_profile::{
    ReviewedSurfaceAudioTablesProfile, ReviewedSurfaceAudioTablesProfileError,
    SURFACE_AUDIO_TABLES_PROFILE_SCHEMA_VERSION, SurfaceAudioTablesEncoding,
    SurfaceAudioTablesExecutable, SurfaceAudioTablesProfile, SurfaceAudioTablesProfileError,
    SurfaceAudioTablesSpec,
};
pub use surface_audio_tables_store::{SurfaceAudioTablesStore, SurfaceAudioTablesStoreError};
pub use surface_import::{
    SURFACE_CAPABILITY_DATA_FILE, SURFACE_CAPABILITY_IMPORT_SCHEMA_VERSION,
    SURFACE_CAPABILITY_STATE_FILE, SURFACE_DATA_FILE, SURFACE_IMPORT_SCHEMA_VERSION,
    SURFACE_STATE_FILE, SurfaceCapabilityImportSummary, SurfaceImportError, SurfaceImportFile,
    SurfaceImportSummary, extract_surface_capabilities, extract_surface_tables,
};
pub use surface_profile::{
    ReviewedSurfaceCapabilityProfile, ReviewedSurfaceProfile, ReviewedSurfaceProfileError,
    SURFACE_CAPABILITY_PROFILE_SCHEMA_VERSION, SurfaceCapabilityEncoding, SurfaceCapabilityProfile,
    SurfaceCapabilitySpec, SurfaceExecutableProfile, SurfaceProfileError, SurfaceTableEncoding,
    SurfaceTableProfile, SurfaceTableSpec,
};
pub use surface_store::{
    SurfaceCapabilityStore, SurfaceDataError, SurfaceTableStore, SurfaceTableStoreError,
    decode_surface_capability_data, decode_surface_table_data,
};
pub use tyre_tables_import::{
    TYRE_TABLES_DATA_FILE, TYRE_TABLES_IMPORT_SCHEMA_VERSION, TYRE_TABLES_STATE_FILE,
    TyreTablesImportError, TyreTablesImportFile, TyreTablesImportShape, TyreTablesImportSummary,
    extract_tyre_surface_tables,
};
pub use tyre_tables_profile::{
    ReviewedTyreTablesProfile, ReviewedTyreTablesProfileError, TYRE_TABLES_PROFILE_SCHEMA_VERSION,
    TyreTablesEncoding, TyreTablesExecutable, TyreTablesProfile, TyreTablesProfileError,
    TyreTablesSpec,
};
pub use tyre_tables_store::{TyreTablesStore, TyreTablesStoreError};
pub use verify::{VerificationError, VerificationSummary, verify_source};
