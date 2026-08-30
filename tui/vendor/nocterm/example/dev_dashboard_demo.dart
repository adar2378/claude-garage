import 'dart:async';
import 'dart:math' as math;
import 'package:nocterm/nocterm.dart';

/// Example dev dashboard - optimized for screenshots.
/// Shows a rich, visually interesting dashboard layout.
void main() async {
  await runApp(const DevDashboard());
}

class DevDashboard extends StatefulComponent {
  const DevDashboard({super.key});

  @override
  State<DevDashboard> createState() => _DevDashboardState();
}

class _DevDashboardState extends State<DevDashboard> {
  Timer? _timer;
  final _random = math.Random();

  // Live metrics
  int _requests = 847;
  int _activeUsers = 23;
  double _responseTime = 42.5;
  double _memoryMb = 256;
  double _cpuPercent = 34;
  final int _errors = 2;

  // Recent requests
  final List<RequestLog> _recentRequests = [
    RequestLog('GET', '/api/users', 200, 23),
    RequestLog('POST', '/api/orders', 201, 89),
    RequestLog('GET', '/api/products/142', 200, 12),
    RequestLog('GET', '/api/analytics', 200, 156),
    RequestLog('PUT', '/api/users/5', 200, 34),
    RequestLog('GET', '/api/health', 200, 3),
    RequestLog('POST', '/api/auth/refresh', 200, 45),
    RequestLog('GET', '/api/config', 304, 8),
  ];

  // Feature flags
  final Map<String, bool> _flags = {
    'Dark Mode': true,
    'New Checkout Flow': true,
    'Beta Features': false,
    'Analytics V2': true,
    'AI Recommendations': false,
  };

  // Quick actions
  final List<QuickAction> _actions = [
    QuickAction('r', 'Restart', true),
    QuickAction('t', 'Run Tests', true),
    QuickAction('b', 'Build APK', true),
    QuickAction('c', 'Clear Cache', true),
    QuickAction('d', 'Debug Paint', false),
    QuickAction('p', 'Perf Overlay', false),
  ];

  @override
  void initState() {
    super.initState();
    _startSimulation();
  }

  @override
  void dispose() {
    _timer?.cancel();
    super.dispose();
  }

  void _startSimulation() {
    _timer = Timer.periodic(Duration(milliseconds: 1200), (_) {
      if (!mounted) return;
      setState(() {
        _requests += 5 + _random.nextInt(15);
        _activeUsers = 20 + _random.nextInt(10);
        _responseTime = 35 + _random.nextDouble() * 30;
        _memoryMb = 240 + _random.nextDouble() * 40;
        _cpuPercent = 25 + _random.nextDouble() * 25;

        // Rotate requests
        _recentRequests.insert(0, _generateRequest());
        if (_recentRequests.length > 8) _recentRequests.removeLast();
      });
    });
  }

  RequestLog _generateRequest() {
    final endpoints = [
      ('GET', '/api/users', 200),
      ('GET', '/api/products', 200),
      ('POST', '/api/orders', 201),
      ('GET', '/api/analytics', 200),
      ('PUT', '/api/cart', 200),
      ('GET', '/api/recommendations', 200),
      ('POST', '/api/events', 202),
    ];
    final e = endpoints[_random.nextInt(endpoints.length)];
    return RequestLog(e.$1, e.$2, e.$3, 5 + _random.nextInt(150));
  }

  @override
  Component build(BuildContext context) {
    return Focusable(
      focused: true,
      onKeyEvent: (e) {
        if (e.logicalKey == LogicalKey.keyQ) {
          shutdownApp();
          return true;
        }
        return false;
      },
      child: Container(
        decoration: BoxDecoration(color: Color(0xFF0d1117)),
        child: Column(
          children: [
            _buildHeader(),
            Expanded(
              child: Container(
                padding: EdgeInsets.all(1),
                child: Row(
                  children: [
                    // Left column
                    Expanded(
                      flex: 3,
                      child: Column(
                        children: [
                          _buildMetricsRow(),
                          SizedBox(height: 1),
                          Expanded(child: _buildRequestsPanel()),
                        ],
                      ),
                    ),
                    SizedBox(width: 1),
                    // Right column
                    Expanded(
                      flex: 2,
                      child: Column(
                        children: [
                          _buildFlagsPanel(),
                          SizedBox(height: 1),
                          Expanded(child: _buildActionsPanel()),
                        ],
                      ),
                    ),
                  ],
                ),
              ),
            ),
            _buildFooter(),
          ],
        ),
      ),
    );
  }

  Component _buildHeader() {
    return Container(
      height: 3,
      decoration: BoxDecoration(
        color: Color(0xFF161b22),
        border: BoxBorder(bottom: BorderSide(color: Color(0xFF30363d))),
      ),
      padding: EdgeInsets.symmetric(horizontal: 2),
      child: Row(
        mainAxisAlignment: MainAxisAlignment.spaceBetween,
        children: [
          Row(children: [
            Text('◆ ', style: TextStyle(color: Color(0xFF58a6ff))),
            Text('MyApp',
                style: TextStyle(
                    color: Color(0xFFf0f6fc), fontWeight: FontWeight.bold)),
            Text(' Dev Dashboard', style: TextStyle(color: Color(0xFF8b949e))),
          ]),
          Row(children: [
            Container(
              padding: EdgeInsets.symmetric(horizontal: 1),
              decoration: BoxDecoration(color: Color(0xFF238636)),
              child: Text(' RUNNING ',
                  style: TextStyle(
                      color: Color(0xFFffffff), fontWeight: FontWeight.bold)),
            ),
            SizedBox(width: 2),
            Text('localhost:8080', style: TextStyle(color: Color(0xFF58a6ff))),
            SizedBox(width: 2),
            Text('development', style: TextStyle(color: Color(0xFF8b949e))),
          ]),
        ],
      ),
    );
  }

  Component _buildMetricsRow() {
    return Row(
      children: [
        Expanded(
            child: _buildMetricCard(
                'Requests', '$_requests', '/min', Color(0xFF58a6ff), '↑ 12%')),
        SizedBox(width: 1),
        Expanded(
            child: _buildMetricCard(
                'Users', '$_activeUsers', 'active', Color(0xFF3fb950), '↑ 3')),
        SizedBox(width: 1),
        Expanded(
            child: _buildMetricCard(
                'Response',
                _responseTime.toStringAsFixed(0),
                'ms avg',
                Color(0xFFd29922),
                '')),
        SizedBox(width: 1),
        Expanded(
            child: _buildMetricCard('Memory', _memoryMb.toStringAsFixed(0),
                'MB', Color(0xFFa371f7), '')),
        SizedBox(width: 1),
        Expanded(
            child: _buildMetricCard('CPU', '${_cpuPercent.toStringAsFixed(0)}%',
                'usage', Color(0xFF8b949e), '')),
        SizedBox(width: 1),
        Expanded(
            child: _buildMetricCard('Errors', '$_errors', 'today',
                _errors > 0 ? Color(0xFFf85149) : Color(0xFF3fb950), '')),
      ],
    );
  }

  Component _buildMetricCard(
      String label, String value, String unit, Color color, String trend) {
    return Container(
      padding: EdgeInsets.all(1),
      decoration: BoxDecoration(
        color: Color(0xFF161b22),
        border: BoxBorder.all(color: Color(0xFF30363d)),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            mainAxisAlignment: MainAxisAlignment.spaceBetween,
            children: [
              Text(label, style: TextStyle(color: Color(0xFF8b949e))),
              if (trend.isNotEmpty)
                Text(trend,
                    style: TextStyle(
                        color: trend.startsWith('↑')
                            ? Color(0xFF3fb950)
                            : Color(0xFFf85149))),
            ],
          ),
          Row(
            crossAxisAlignment: CrossAxisAlignment.end,
            children: [
              Text(value,
                  style: TextStyle(color: color, fontWeight: FontWeight.bold)),
              Text(' $unit', style: TextStyle(color: Color(0xFF484f58))),
            ],
          ),
        ],
      ),
    );
  }

  Component _buildRequestsPanel() {
    return Container(
      decoration: BoxDecoration(
        color: Color(0xFF161b22),
        border: BoxBorder.all(color: Color(0xFF30363d)),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Container(
            padding: EdgeInsets.symmetric(horizontal: 1, vertical: 1),
            decoration: BoxDecoration(
              border: BoxBorder(bottom: BorderSide(color: Color(0xFF30363d))),
            ),
            child: Row(
              mainAxisAlignment: MainAxisAlignment.spaceBetween,
              children: [
                Text('Recent Requests',
                    style: TextStyle(
                        color: Color(0xFFf0f6fc), fontWeight: FontWeight.bold)),
                Text('Live', style: TextStyle(color: Color(0xFF3fb950))),
              ],
            ),
          ),
          Expanded(
            child: Container(
              padding: EdgeInsets.all(1),
              child: Column(
                children: [
                  // Header row
                  Container(
                    padding: EdgeInsets.only(bottom: 1),
                    decoration: BoxDecoration(
                      border: BoxBorder(
                          bottom: BorderSide(color: Color(0xFF21262d))),
                    ),
                    child: Row(children: [
                      SizedBox(
                          width: 8,
                          child: Text('Method',
                              style: TextStyle(color: Color(0xFF484f58)))),
                      Expanded(
                          child: Text('Endpoint',
                              style: TextStyle(color: Color(0xFF484f58)))),
                      SizedBox(
                          width: 8,
                          child: Text('Status',
                              style: TextStyle(color: Color(0xFF484f58)))),
                      SizedBox(
                          width: 10,
                          child: Text('Time',
                              style: TextStyle(color: Color(0xFF484f58)))),
                    ]),
                  ),
                  // Request rows
                  for (final req in _recentRequests)
                    Container(
                      padding: EdgeInsets.symmetric(vertical: 0),
                      child: Row(children: [
                        SizedBox(
                          width: 8,
                          child: Text(
                            req.method,
                            style: TextStyle(
                              color: req.method == 'GET'
                                  ? Color(0xFF3fb950)
                                  : req.method == 'POST'
                                      ? Color(0xFF58a6ff)
                                      : Color(0xFFd29922),
                              fontWeight: FontWeight.bold,
                            ),
                          ),
                        ),
                        Expanded(
                          child: Text(req.endpoint,
                              style: TextStyle(color: Color(0xFFc9d1d9))),
                        ),
                        SizedBox(
                          width: 8,
                          child: Text(
                            '${req.status}',
                            style: TextStyle(
                              color: req.status < 300
                                  ? Color(0xFF3fb950)
                                  : req.status < 400
                                      ? Color(0xFF8b949e)
                                      : Color(0xFFf85149),
                            ),
                          ),
                        ),
                        SizedBox(
                          width: 10,
                          child: Text(
                            '${req.timeMs}ms',
                            style: TextStyle(
                              color: req.timeMs < 50
                                  ? Color(0xFF3fb950)
                                  : req.timeMs < 100
                                      ? Color(0xFFd29922)
                                      : Color(0xFFf85149),
                            ),
                          ),
                        ),
                      ]),
                    ),
                ],
              ),
            ),
          ),
        ],
      ),
    );
  }

  Component _buildFlagsPanel() {
    return Container(
      decoration: BoxDecoration(
        color: Color(0xFF161b22),
        border: BoxBorder.all(color: Color(0xFF30363d)),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Container(
            padding: EdgeInsets.symmetric(horizontal: 1, vertical: 1),
            decoration: BoxDecoration(
              border: BoxBorder(bottom: BorderSide(color: Color(0xFF30363d))),
            ),
            child: Row(
              mainAxisAlignment: MainAxisAlignment.spaceBetween,
              children: [
                Text('Feature Flags',
                    style: TextStyle(
                        color: Color(0xFFf0f6fc), fontWeight: FontWeight.bold)),
                Text('${_flags.values.where((v) => v).length}/${_flags.length}',
                    style: TextStyle(color: Color(0xFF8b949e))),
              ],
            ),
          ),
          Container(
            padding: EdgeInsets.all(1),
            child: Column(
              children: [
                for (final entry in _flags.entries)
                  Container(
                    padding: EdgeInsets.symmetric(vertical: 0),
                    child: Row(children: [
                      Text(
                        entry.value ? '●' : '○',
                        style: TextStyle(
                            color: entry.value
                                ? Color(0xFF3fb950)
                                : Color(0xFF484f58)),
                      ),
                      SizedBox(width: 1),
                      Expanded(
                        child: Text(
                          entry.key,
                          style: TextStyle(
                              color: entry.value
                                  ? Color(0xFFc9d1d9)
                                  : Color(0xFF8b949e)),
                        ),
                      ),
                      Text(
                        entry.value ? 'ON' : 'OFF',
                        style: TextStyle(
                          color: entry.value
                              ? Color(0xFF3fb950)
                              : Color(0xFF484f58),
                          fontWeight: FontWeight.bold,
                        ),
                      ),
                    ]),
                  ),
              ],
            ),
          ),
        ],
      ),
    );
  }

  Component _buildActionsPanel() {
    return Container(
      decoration: BoxDecoration(
        color: Color(0xFF161b22),
        border: BoxBorder.all(color: Color(0xFF30363d)),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Container(
            padding: EdgeInsets.symmetric(horizontal: 1, vertical: 1),
            decoration: BoxDecoration(
              border: BoxBorder(bottom: BorderSide(color: Color(0xFF30363d))),
            ),
            child: Text('Quick Actions',
                style: TextStyle(
                    color: Color(0xFFf0f6fc), fontWeight: FontWeight.bold)),
          ),
          Expanded(
            child: Container(
              padding: EdgeInsets.all(1),
              child: Column(
                children: [
                  for (final action in _actions)
                    Container(
                      padding: EdgeInsets.symmetric(vertical: 0),
                      child: Row(children: [
                        Container(
                          width: 3,
                          decoration: BoxDecoration(
                            color: Color(0xFF21262d),
                            border: BoxBorder.all(color: Color(0xFF30363d)),
                          ),
                          child: Center(
                            child: Text(action.key,
                                style: TextStyle(color: Color(0xFF58a6ff))),
                          ),
                        ),
                        SizedBox(width: 1),
                        Expanded(
                          child: Text(action.label,
                              style: TextStyle(color: Color(0xFFc9d1d9))),
                        ),
                        if (action.isAction)
                          Text('→', style: TextStyle(color: Color(0xFF484f58)))
                        else
                          Text(
                            'OFF',
                            style: TextStyle(color: Color(0xFF484f58)),
                          ),
                      ]),
                    ),
                  Spacer(),
                  Container(
                    padding: EdgeInsets.all(1),
                    decoration: BoxDecoration(
                      color: Color(0xFF0d1117),
                      border: BoxBorder.all(color: Color(0xFF238636)),
                    ),
                    child: Row(children: [
                      Text('VM Service ',
                          style: TextStyle(color: Color(0xFF8b949e))),
                      Text('Connected',
                          style: TextStyle(color: Color(0xFF3fb950))),
                    ]),
                  ),
                ],
              ),
            ),
          ),
        ],
      ),
    );
  }

  Component _buildFooter() {
    return Container(
      height: 2,
      decoration: BoxDecoration(
        color: Color(0xFF161b22),
        border: BoxBorder(top: BorderSide(color: Color(0xFF30363d))),
      ),
      padding: EdgeInsets.symmetric(horizontal: 2),
      child: Row(
        mainAxisAlignment: MainAxisAlignment.spaceBetween,
        children: [
          Text('[Q] Quit  [R] Restart  [T] Tests  [B] Build',
              style: TextStyle(color: Color(0xFF484f58))),
          Row(children: [
            Text('nocterm', style: TextStyle(color: Color(0xFF484f58))),
            Text(' v0.4.4', style: TextStyle(color: Color(0xFF30363d))),
          ]),
        ],
      ),
    );
  }
}

class RequestLog {
  final String method;
  final String endpoint;
  final int status;
  final int timeMs;
  RequestLog(this.method, this.endpoint, this.status, this.timeMs);
}

class QuickAction {
  final String key;
  final String label;
  final bool isAction;
  QuickAction(this.key, this.label, this.isAction);
}
