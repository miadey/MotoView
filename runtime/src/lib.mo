/// MotoView runtime — package entry point.
///
///   import MV "mo:motoview";        // types + helpers
///   import App "mo:motoview/App";   // the runtime orchestrator
///   import Html "mo:motoview/Html"; // the render Builder
///
/// Rendering is a query, events are updates, and the browser synchronizes
/// through versioned UI batches. See `App.mo` for the orchestrator.
import Types "Types";
import ClientAssets "ClientAssets";

module {
  public type HttpRequest = Types.HttpRequest;
  public type HttpResponse = Types.HttpResponse;
  public type HeaderField = Types.HeaderField;
  public type Ctx = Types.Ctx;
  public type Head = Types.Head;
  public type Effect = Types.Effect;
  public type Batch = Types.Batch;
  public type BatchStatus = Types.BatchStatus;
  public type Page = Types.Page;
  public type Layout = Types.Layout;
  public type Config = Types.Config;
  public type Assets = Types.Assets;

  public func emptyHead() : Head { Types.emptyHead() };

  /// Default client assets (the Rust→WASM bridge + bootstrap + CSS).
  public func defaultAssets() : Assets { ClientAssets.assets() };
};
