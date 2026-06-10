/// Minimal JSON *encoding* for render batches (protocol "motoview/1").
///
/// The runtime only ever *produces* JSON (responses); it never parses JSON
/// (events arrive form-encoded), so no decoder is needed.
import Text "mo:base/Text";
import Buffer "mo:base/Buffer";
import Types "Types";

module {

  /// Escape a Text as a JSON string body (without surrounding quotes).
  public func escape(t : Text) : Text {
    var s = t;
    s := Text.replace(s, #char '\\', "\\\\");
    s := Text.replace(s, #char '\"', "\\\"");
    s := Text.replace(s, #char '\n', "\\n");
    s := Text.replace(s, #char '\r', "\\r");
    s := Text.replace(s, #char '\t', "\\t");
    s;
  };

  func str(t : Text) : Text { "\"" # escape(t) # "\"" };

  func encodeHead(h : Types.Head) : Text {
    "{\"title\":" # str(h.title)
    # ",\"description\":" # str(h.description)
    # ",\"canonical\":" # str(h.canonical) # "}";
  };

  func encodeEffects(effects : [Types.Effect]) : Text {
    let parts = Buffer.Buffer<Text>(effects.size());
    for (e in effects.vals()) {
      parts.add("{\"type\":" # str(e.kind) # ",\"target\":" # str(e.target) # ",\"value\":" # str(e.value) # "}");
    };
    "[" # Text.join(",", parts.vals()) # "]";
  };

  func encodePairs(pairs : [(Text, Text)]) : Text {
    let parts = Buffer.Buffer<Text>(pairs.size());
    for ((k, v) in pairs.vals()) { parts.add(str(k) # ":" # str(v)) };
    "{" # Text.join(",", parts.vals()) # "}";
  };

  // The optional portable UI-IR field: `,"ui":<json>` when present, else "".
  // The IR is already JSON, so it is embedded verbatim (not re-stringified).
  func uiField(ui : ?Text) : Text {
    switch (ui) {
      case (?j) { ",\"ui\":" # j };
      case null { "" };
    };
  };

  /// Encode a full batch to the motoview/1 JSON wire format.
  public func encodeBatch(b : Types.Batch) : Text {
    switch (b.status) {
      case (#unchanged) {
        "{\"protocol\":\"motoview/1\",\"status\":\"unchanged\",\"batchId\":" # str(b.batchId) # "}";
      };
      case (#redirect) {
        "{\"protocol\":\"motoview/1\",\"status\":\"redirect\",\"location\":" # str(b.location) # "}";
      };
      case (#validationError) {
        "{\"protocol\":\"motoview/1\",\"status\":\"validation-error\""
        # ",\"batchId\":" # str(b.batchId)
        # ",\"target\":" # str(b.target)
        # ",\"html\":" # str(b.html)
        # uiField(b.ui)
        # ",\"errors\":" # encodePairs(b.errors)
        # ",\"effects\":" # encodeEffects(b.effects) # "}";
      };
      case (#changed) {
        "{\"protocol\":\"motoview/1\",\"status\":\"changed\""
        # ",\"batchId\":" # str(b.batchId)
        # ",\"mode\":\"replace\""
        # ",\"target\":" # str(b.target)
        # ",\"html\":" # str(b.html)
        # uiField(b.ui)
        # ",\"head\":" # encodeHead(b.head)
        # ",\"effects\":" # encodeEffects(b.effects) # "}";
      };
    };
  };
};
