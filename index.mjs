// ESM wrapper for CommonJS native addon
import { createRequire } from 'node:module';
const require = createRequire(import.meta.url);

// Import the native addon using CommonJS require
const nativeAddon = require('./index.js');

// Re-export everything
export const {
  DicomFile,
  FindScu,
  MoveScu,
  QidoInstanceResult,
  QidoSeriesResult,
  QidoStudyResult,
  QidoServer,
  QueryBuilder,
  StoreScp,
  StoreScu,
  WadoServer,
  AbstractSyntaxMode,
  MoveQueryModel,
  PixelDataFormat,
  QueryModel,
  ResultStatus,
  StorageBackend,
  StorageBackendType,
  StoreScpEvent,
  StoreScuEvent,
  TagScope,
  TransferSyntaxMode,
  WadoMediaType,
  WadoStorageType,
  WadoTranscoding,
  createQidoEmptyResponse,
  createQidoInstancesResponse,
  createQidoSeriesResponse,
  createQidoStudiesResponse,
  createCustomTag,
  getAvailableTagNames,
  getCommonSopClasses,
  getCommonTagSets,
  getCommonTransferSyntaxes,
  combineTags
} = nativeAddon;

export default nativeAddon;
