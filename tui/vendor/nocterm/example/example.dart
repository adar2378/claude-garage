// A simple todo app built with nocterm.
// Run with: dart run example/example.dart

import 'package:nocterm/nocterm.dart';

void main() {
  runApp(const TodoApp());
}

class TodoApp extends StatefulComponent {
  const TodoApp({super.key});

  @override
  State<TodoApp> createState() => _TodoAppState();
}

class _TodoAppState extends State<TodoApp> {
  final List<TodoItem> todos = [
    TodoItem('Learn nocterm basics', true),
    TodoItem('Build a TUI application', true),
    TodoItem('Add keyboard navigation', false),
    TodoItem('Implement state management', false),
    TodoItem('Deploy', false),
  ];
  int selectedIndex = 0;

  @override
  Component build(BuildContext context) {
    return Focusable(
      focused: true,
      onKeyEvent: (key) {
        if (key.logicalKey == LogicalKey.arrowDown) {
          setState(() {
            if (selectedIndex < todos.length - 1) selectedIndex++;
          });
          return true;
        } else if (key.logicalKey == LogicalKey.arrowUp) {
          setState(() {
            if (selectedIndex > 0) selectedIndex--;
          });
          return true;
        } else if (key.logicalKey == LogicalKey.space) {
          setState(() {
            todos[selectedIndex].completed = !todos[selectedIndex].completed;
          });
          return true;
        }
        return false;
      },
      child: Container(
        padding: const EdgeInsets.all(2),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Center(
              child: Text(
                'Terminal Todo App',
                style: TextStyle(
                  fontWeight: FontWeight.bold,
                  color: Colors.yellow,
                ),
              ),
            ),
            const SizedBox(height: 1),
            Row(
              mainAxisAlignment: MainAxisAlignment.spaceAround,
              children: [
                Text(
                  'Total: ${todos.length}',
                  style: TextStyle(color: Colors.brightBlue),
                ),
                Text(
                  'Done: ${todos.where((t) => t.completed).length}',
                  style: TextStyle(color: Colors.brightGreen),
                ),
              ],
            ),
            const SizedBox(height: 1),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  for (int i = 0; i < todos.length; i++)
                    _buildTodoItem(todos[i], i == selectedIndex),
                ],
              ),
            ),
            const SizedBox(height: 1),
            Center(
              child: Text(
                'Arrow keys: Navigate | Space: Toggle',
                style: TextStyle(color: Colors.brightBlack),
              ),
            ),
          ],
        ),
      ),
    );
  }

  Component _buildTodoItem(TodoItem todo, bool isSelected) {
    final checkmark = todo.completed ? '[x]' : '[ ]';
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 1),
      decoration:
          isSelected ? BoxDecoration(color: Color.fromRGB(30, 40, 50)) : null,
      child: Row(
        children: [
          Text(
            isSelected ? '> ' : '  ',
            style: TextStyle(color: Colors.brightCyan),
          ),
          Text(
            '$checkmark ',
            style: TextStyle(
              color: todo.completed ? Colors.brightGreen : Colors.brightYellow,
            ),
          ),
          Expanded(
            child: Text(
              todo.text,
              style: todo.completed
                  ? TextStyle(
                      color: Colors.brightBlack,
                      decoration: TextDecoration.lineThrough,
                    )
                  : TextStyle(
                      color: isSelected ? Colors.brightWhite : Colors.white,
                    ),
            ),
          ),
        ],
      ),
    );
  }
}

class TodoItem {
  final String text;
  bool completed;

  TodoItem(this.text, this.completed);
}
