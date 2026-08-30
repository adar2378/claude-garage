import 'package:nocterm/src/foundation/persistent_hash_map.dart';
import 'package:test/test.dart';

/// Direct unit tests for the persistent HAMT used by the framework for
/// inherited-component lookups. Exercises the trie internals (deep levels,
/// node inflation, collision nodes) that incidental framework usage
/// never reaches.
void main() {
  group('PersistentHashMap', () {
    test('empty map returns null for any key', () {
      const map = PersistentHashMap<String, int>.empty();
      expect(map['missing'], isNull);
    });

    test('put/get roundtrip across enough keys to build a deep trie', () {
      var map = const PersistentHashMap<String, int>.empty();
      const n = 2000;
      for (var i = 0; i < n; i++) {
        map = map.put('key-$i', i);
      }
      for (var i = 0; i < n; i++) {
        expect(map['key-$i'], i, reason: 'key-$i must survive $n inserts');
      }
      expect(map['key-$n'], isNull);
    });

    test('put overwrites and putting an identical value is a no-op copy', () {
      var map = const PersistentHashMap<String, String>.empty();
      map = map.put('k', 'a');
      map = map.put('k', 'b');
      expect(map['k'], 'b');

      // Identical (key, value) put returns the same instance - the
      // structural-sharing fast path.
      final same = map.put('k', 'b');
      expect(identical(same, map), isTrue);
    });

    test('puts are persistent - earlier versions are unchanged', () {
      var v1 = const PersistentHashMap<String, int>.empty();
      for (var i = 0; i < 50; i++) {
        v1 = v1.put('k$i', i);
      }
      final v2 = v1.put('k0', 999).put('new', 1);

      expect(v1['k0'], 0, reason: 'old version must not see the overwrite');
      expect(v1['new'], isNull);
      expect(v2['k0'], 999);
      expect(v2['new'], 1);
      // Untouched keys shared by both versions.
      expect(v1['k49'], 49);
      expect(v2['k49'], 49);
    });

    test('handles hash collisions', () {
      var map = const PersistentHashMap<_Colliding, String>.empty();
      const a = _Colliding('a');
      const b = _Colliding('b');
      const c = _Colliding('c');

      map = map.put(a, 'A').put(b, 'B').put(c, 'C');
      expect(map[a], 'A');
      expect(map[b], 'B');
      expect(map[c], 'C');

      // Overwrite inside the collision node.
      map = map.put(b, 'B2');
      expect(map[a], 'A');
      expect(map[b], 'B2');
      expect(map[c], 'C');

      expect(map[const _Colliding('d')], isNull,
          reason: 'same hash but unequal key must miss');
    });

    test('from(map) copies all entries', () {
      final source = {for (var i = 0; i < 100; i++) 'k$i': i};
      final map = PersistentHashMap<String, int>.from(source);
      for (final entry in source.entries) {
        expect(map[entry.key], entry.value);
      }
    });
  });
}

/// All instances share one hashCode; equality is by name.
class _Colliding {
  const _Colliding(this.name);
  final String name;

  @override
  bool operator ==(Object other) => other is _Colliding && other.name == name;

  @override
  int get hashCode => 42;
}
