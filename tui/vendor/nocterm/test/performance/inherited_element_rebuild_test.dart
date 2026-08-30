import 'package:nocterm/nocterm.dart';
import 'package:test/test.dart';

class BuildTracker extends StatelessComponent {
  final VoidCallback onBuild;
  const BuildTracker({required this.onBuild, super.key});

  @override
  Component build(BuildContext context) {
    onBuild();
    return const SizedBox();
  }
}

class MyDataComponent extends InheritedComponent {
  final int value;

  const MyDataComponent({
    required this.value,
    required super.child,
  });

  @override
  bool updateShouldNotify(MyDataComponent old) => value != old.value;
}

void main() {
  test('Standard InheritedComponent triggers redundant builds', () {
    int builds = 0;
    final tracker = BuildTracker(onBuild: () => builds++);

    // Initial Mount
    final first = MyDataComponent(value: 100, child: tracker);
    final element = first.createElement();
    element.mount(null, null);

    builds = 0; // Reset after initial mount

    // Update with identical data.. NO REBUILD NEEDED
    final second = MyDataComponent(value: 100, child: tracker);
    element.update(second);

    expect(
      builds,
      0,
      reason: 'Subtree should not rebuild if updateShouldNotify is false',
    );
  });
}
