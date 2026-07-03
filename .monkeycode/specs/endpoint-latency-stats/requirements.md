# Requirements Document

## Introduction

Endpoint Latency Statistics and Leaderboard feature provides visibility into response latency per upstream LLM API endpoint. The system collects per-request duration, computes aggregate latency metrics, and exposes a leaderboard for quick comparison.

## Glossary

- **Endpoint**: A configured upstream LLM API provider registered in TokenHub.
- **Duration**: The total round-trip time (milliseconds) spent on a single upstream request, measured from request start to response completion.
- **Leaderboard**: A ranked view of endpoints ordered by average latency.

## Requirements

### Requirement 1: Latency Collection

**User Story:** AS an operator, I want per-endpoint latency to be recorded automatically, so that I can identify slow endpoints.

#### Acceptance Criteria

1. WHEN the proxy completes a request to an upstream endpoint, the system SHALL record the endpoint ID and the request duration.
2. WHEN a request fails due to network or HTTP error, the system SHALL still record the duration if the endpoint was reached.
3. WHEN the latency sample buffer for an endpoint reaches 100 entries, the system SHALL discard the oldest sample.

### Requirement 2: Latency Aggregation

**User Story:** AS an operator, I want aggregate latency metrics per endpoint, so that I can compare endpoint performance without analyzing every request.

#### Acceptance Criteria

1. WHEN the operator requests endpoint latency metrics, the system SHALL return the average, minimum, maximum, P50, P90, and P95 durations for each endpoint.
2. WHEN an endpoint has no recorded samples, the system SHALL return empty latency metrics for that endpoint.

### Requirement 3: Latency Leaderboard

**User Story:** AS an operator, I want a leaderboard sorted by average latency, so that I can quickly identify the fastest and slowest endpoints.

#### Acceptance Criteria

1. WHEN the operator opens the latency leaderboard page, the system SHALL display all endpoints ranked by average latency in ascending order.
2. WHEN an endpoint is disabled, the system SHALL still include the endpoint in the leaderboard and mark the disabled status.
3. WHEN the operator refreshes the page, the system SHALL reload the latest latency metrics.

### Requirement 4: Frontend Display

**User Story:** AS an operator, I want a dedicated page for latency statistics, so that I can access the leaderboard alongside endpoint management.

#### Acceptance Criteria

1. WHEN the operator clicks the "Latency Leaderboard" navigation item, the system SHALL display the latency leaderboard tab.
2. WHEN the leaderboard loads, the system SHALL show each endpoint name, status, and average/min/max/P95 latency.
3. WHEN the operator hovers over a metric, the system MAY display the exact millisecond value.

