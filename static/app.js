// API 基础路径
const API_BASE = '/admin/api';

// 状态
let currentEndpoints = [];
let currentConfig = {};
let currentPools = [];
let currentApis = [];

// 初始化
document.addEventListener('DOMContentLoaded', () => {
    checkAuth();
    initEventListeners();
});

// 检查登录状态
async function checkAuth() {
    try {
        const res = await fetch(`${API_BASE}/auth/status`);
        const data = await res.json();
        if (data.authenticated) {
            showMainPage();
            loadDashboard();
        } else {
            showLoginPage();
        }
    } catch (e) {
        showLoginPage();
    }
}

// ========== 模型映射管理 ==========

// 更新模型映射区域的显示/隐藏
function updateModelMappingsVisibility(fromPool = false) {
    const poolId = document.getElementById('ep-pool-id').value;
    const mappingsGroup = document.getElementById('model-mappings-group');
    
    if (!poolId || !mappingsGroup || !fromPool) {
        if (mappingsGroup) mappingsGroup.style.display = 'none';
        return;
    }
    
    // 查找池的模型模式
    const pool = currentPools.find(p => p.id === poolId);
    if (pool && pool.model_mode === 'mapping') {
        mappingsGroup.style.display = 'block';
    } else {
        mappingsGroup.style.display = 'none';
    }
}

// 添加模型映射行
function addModelMappingRow(clientModel, endpointModel, models = []) {
    const container = document.getElementById('model-mappings-list');
    if (!container) return;
    
    // 如果没有传入模型列表，尝试从容器的 data 属性获取
    if (models.length === 0) {
        models = container.dataset.models ? JSON.parse(container.dataset.models) : [];
    }
    
    // 构建模型选项
    let modelOptions = '<option value="">选择端点模型</option>';
    models.forEach(m => {
        const selected = m === endpointModel ? 'selected' : '';
        modelOptions += `<option value="${escapeAttr(m)}" ${selected}>${escapeHtml(m)}</option>`;
    });
    
    const row = document.createElement('div');
    row.style.cssText = 'display: flex; gap: 8px; margin-bottom: 8px; align-items: center;';
    row.innerHTML = `
        <input type="text" class="mapping-client-model" placeholder="客户端模型名" value="${escapeHtml(clientModel)}" style="flex: 1;">
        <span style="color: var(--text-tertiary);">→</span>
        <select class="mapping-endpoint-model" style="flex: 1;">
            ${modelOptions}
        </select>
        <button type="button" class="btn btn-small btn-danger" onclick="this.parentElement.remove()">删除</button>
    `;
    container.appendChild(row);
}

// 获取模型映射数据
function getModelMappings() {
    const container = document.getElementById('model-mappings-list');
    if (!container) return [];
    
    const mappings = [];
    const rows = container.querySelectorAll('div');
    rows.forEach(row => {
        const clientModel = row.querySelector('.mapping-client-model')?.value?.trim();
        const endpointModel = row.querySelector('.mapping-endpoint-model')?.value?.trim();
        if (clientModel && endpointModel) {
            mappings.push({ client_model: clientModel, endpoint_model: endpointModel });
        }
    });
    return mappings;
}

// 加载模型映射数据
function loadModelMappings(mappings, models = []) {
    const container = document.getElementById('model-mappings-list');
    if (!container) return;
    
    // 存储模型列表到容器的 data 属性
    container.dataset.models = JSON.stringify(models);
    
    container.innerHTML = '';
    if (mappings && mappings.length > 0) {
        mappings.forEach(m => addModelMappingRow(m.client_model, m.endpoint_model, models));
    }
}

// 更新端点完整路径显示
function updateEndpointFullUrl() {
    const epUrl = document.getElementById('ep-url');
    const epType = document.getElementById('ep-type');
    const fullUrlDiv = document.getElementById('ep-full-url');
    
    if (!epUrl || !epType || !fullUrlDiv) return;
    
    const baseUrl = epUrl.value.trim();
    const apiType = epType.value;
    
    if (!baseUrl) {
        fullUrlDiv.textContent = '';
        return;
    }
    
    // 根据接口类型确定端点路径
    let endpoint = '';
    switch (apiType) {
        case 'openai':
            endpoint = '/chat/completions';
            break;
        case 'anthropic':
            endpoint = '/messages';
            break;
        case 'openai-responses':
            endpoint = '/responses';
            break;
        default:
            endpoint = '/chat/completions';
    }
    
    const cleanBase = baseUrl.replace(/\/+$/, '');
    
    // 检查 URL 路径中是否已包含版本前缀（如 /v1, /v6 等）
    // 如果已有版本前缀，直接拼接端点；否则添加 /v1
    const urlPath = new URL(cleanBase).pathname;
    const hasVersionPrefix = /\/v\d+/.test(urlPath);
    
    const fullUrl = hasVersionPrefix
        ? cleanBase + endpoint
        : cleanBase + '/v1' + endpoint;
    
    fullUrlDiv.textContent = '完整路径: ' + fullUrl;
}

// 初始化事件监听
function initEventListeners() {
    // 登录表单
    document.getElementById('login-form').addEventListener('submit', handleLogin);

    // 导航切换
    document.querySelectorAll('.nav-btn').forEach(btn => {
        btn.addEventListener('click', () => switchTab(btn.dataset.tab));
    });

    // 登出
    document.getElementById('btn-logout').addEventListener('click', handleLogout);

    // 调用日志刷新
    const btnRefreshLogs = document.getElementById('btn-refresh-logs');
    if (btnRefreshLogs) {
        btnRefreshLogs.addEventListener('click', loadCallLogs);
    }

    // 延迟排行榜刷新
    const btnRefreshLatency = document.getElementById('btn-refresh-latency');
    if (btnRefreshLatency) {
        btnRefreshLatency.addEventListener('click', loadLatencyLeaderboard);
    }

    // 密码表单
    document.getElementById('password-form').addEventListener('submit', handleChangePassword);

    // 端点表单
    document.getElementById('endpoint-form').addEventListener('submit', handleSaveEndpoint);

    // 监听 URL 和接口类型变化，更新完整路径显示
    const epUrl = document.getElementById('ep-url');
    const epType = document.getElementById('ep-type');
    if (epUrl && epType) {
        epUrl.addEventListener('input', updateEndpointFullUrl);
        epType.addEventListener('change', updateEndpointFullUrl);
    }

    // 监听限额变化，控制重置方式
    const epLimit = document.getElementById('ep-limit');
    const epReset = document.getElementById('ep-reset');
    const epResetHint = document.getElementById('ep-reset-hint');
    if (epLimit && epReset) {
        const updateResetPolicy = () => {
            if (!epLimit.value || epLimit.value === '0') {
                // 限额为空时，固定为手动重置并禁用
                epReset.value = 'manual';
                epReset.disabled = true;
                if (epResetHint) epResetHint.style.display = 'block';
            } else {
                // 限额不为空时，启用选择
                epReset.disabled = false;
                if (epResetHint) epResetHint.style.display = 'none';
            }
        };
        epLimit.addEventListener('input', updateResetPolicy);
        // 初始化时也检查一次
        updateResetPolicy();
    }

    // 监听请求次数限制变化，控制重置方式
    const epReqLimit = document.getElementById('ep-req-limit');
    const epReqReset = document.getElementById('ep-req-reset');
    const epReqResetHint = document.getElementById('ep-req-reset-hint');
    if (epReqLimit && epReqReset) {
        const updateReqResetPolicy = () => {
            if (!epReqLimit.value || epReqLimit.value === '0') {
                // 请求限制为空时，固定为手动重置并禁用
                epReqReset.value = 'manual';
                epReqReset.disabled = true;
                if (epReqResetHint) epReqResetHint.style.display = 'block';
            } else {
                // 请求限制不为空时，启用选择
                epReqReset.disabled = false;
                if (epReqResetHint) epReqResetHint.style.display = 'none';
            }
        };
        epReqLimit.addEventListener('input', updateReqResetPolicy);
        // 初始化时也检查一次
        updateReqResetPolicy();
    }

    // 添加模型映射按钮
    const btnAddMapping = document.getElementById('btn-add-mapping');
    if (btnAddMapping) {
        btnAddMapping.addEventListener('click', () => {
            addModelMappingRow('', '');
        });
    }

    // 监听端点池选择变化，控制模型映射显示
    const epPoolId = document.getElementById('ep-pool-id');
    if (epPoolId) {
        epPoolId.addEventListener('change', updateModelMappingsVisibility);
    }

    // 浏览模型按钮（表单内）
    const btnBrowseModelsForm = document.getElementById('btn-browse-models-form');
    if (btnBrowseModelsForm) {
        btnBrowseModelsForm.addEventListener('click', handleBrowseModelsForm);
    }

    // 池一键测试开始按钮
    const btnStartPoolTest = document.getElementById('btn-start-pool-test');
    if (btnStartPoolTest) {
        btnStartPoolTest.addEventListener('click', startPoolTest);
    }

    // 对话测试按钮
    document.getElementById('btn-check-endpoint').addEventListener('click', handleCheckEndpoint);

    // 确认模型选择按钮
    const btnConfirmModel = document.getElementById('btn-confirm-model');
    if (btnConfirmModel) {
        btnConfirmModel.addEventListener('click', () => {
            const container = document.getElementById('models-list');
            if (container && container.dataset.apiData) {
                // 对外接口测试
                confirmApiModelAndTest();
            } else {
                // 端点测试
                confirmModelAndTest();
            }
        });
    }

    // 设置页面的修改密码按钮
    const btnChangePwdSettings = document.getElementById('btn-change-password-settings');
    if (btnChangePwdSettings) {
        btnChangePwdSettings.addEventListener('click', () => {
            showModal('password-modal');
        });
    }

    // 重置所有
    document.getElementById('btn-reset-all').addEventListener('click', handleResetAll);

    // 添加端点按钮（端点列表页面）
    const btnAddEndpoint = document.getElementById('btn-add-endpoint');
    if (btnAddEndpoint) {
        btnAddEndpoint.addEventListener('click', () => {
            addEndpointToPool('');
        });
    }

    // 模型搜索框
    const modelSearch = document.getElementById('model-search');
    if (modelSearch) {
        modelSearch.addEventListener('input', (e) => {
            searchModels(e.target.value);
        });
    }

    // 端点搜索框（选择端点到池）
    const endpointSearch = document.getElementById('endpoint-search');
    if (endpointSearch) {
        endpointSearch.addEventListener('input', (e) => {
            searchEndpointsForPool(e.target.value);
        });
    }

    // 确认添加端点到池按钮
    const btnConfirmAddEndpoints = document.getElementById('btn-confirm-add-endpoints');
    if (btnConfirmAddEndpoints) {
        btnConfirmAddEndpoints.addEventListener('click', confirmAddEndpointsToPool);
    }

    // 添加端点映射按钮
    const btnAddEndpointMapping = document.getElementById('btn-add-endpoint-mapping');
    if (btnAddEndpointMapping) {
        btnAddEndpointMapping.addEventListener('click', addEndpointMappingRow);
    }

    // 保存端点映射按钮
    const btnSaveEndpointMapping = document.getElementById('btn-save-endpoint-mapping');
    if (btnSaveEndpointMapping) {
        btnSaveEndpointMapping.addEventListener('click', saveEndpointMapping);
    }

    // 添加对外API
    document.getElementById('btn-add-api').addEventListener('click', () => {
        document.getElementById('api-modal-title').textContent = '添加对外接口';
        document.getElementById('api-form').reset();
        document.getElementById('api-id').value = '';
        document.getElementById('api-enabled').checked = true;
        // 清空完整 URL 显示
        const apiFullUrlDiv = document.getElementById('api-full-url');
        if (apiFullUrlDiv) {
            apiFullUrlDiv.textContent = '';
        }
        // 清空测试结果
        const apiTestResult = document.getElementById('api-test-result');
        if (apiTestResult) {
            apiTestResult.style.display = 'none';
        }
        loadPoolOptions('api-pool');
        showModal('api-modal');
    });

    // 对外API表单
    document.getElementById('api-form').addEventListener('submit', handleSaveApi);

    // 对外接口对话测试按钮
    const btnTestApi = document.getElementById('btn-test-api');
    if (btnTestApi) {
        btnTestApi.addEventListener('click', handleTestApi);
    }

    // 监听对外接口 URL 前缀变化，更新完整调用 URL
    const apiPrefix = document.getElementById('api-prefix');
    const apiType = document.getElementById('api-type');
    if (apiPrefix && apiType) {
        apiPrefix.addEventListener('input', updateApiFullUrlDisplay);
        apiType.addEventListener('change', updateApiFullUrlDisplay);
    }

    // 添加池
    document.getElementById('btn-add-pool').addEventListener('click', () => {
        document.getElementById('pool-modal-title').textContent = '添加端点池';
        document.getElementById('pool-form').reset();
        document.getElementById('pool-id').value = '';
        showModal('pool-modal');
    });

    // 池表单
    document.getElementById('pool-form').addEventListener('submit', handleSavePool);

    // 池调度算法切换说明
    const poolAlgoSelect = document.getElementById('pool-algorithm');
    if (poolAlgoSelect) {
        poolAlgoSelect.addEventListener('change', () => updatePoolAlgoDescription());
    }
    
    // 模型模式切换说明
    const poolModelModeSelect = document.getElementById('pool-model-mode');
    if (poolModelModeSelect) {
        poolModelModeSelect.addEventListener('change', () => updateModelModeDescription());
    }
    
    // 重试模式切换说明
    const poolRetryModeSelect = document.getElementById('pool-retry-mode');
    if (poolRetryModeSelect) {
        poolRetryModeSelect.addEventListener('change', () => updateRetryModeDescription());
    }

    // 移动端侧边栏切换
    const menuToggle = document.getElementById('menu-toggle');
    const sidebar = document.querySelector('.sidebar');
    if (menuToggle && sidebar) {
        menuToggle.addEventListener('click', (e) => {
            e.stopPropagation();
            sidebar.classList.toggle('open');
        });

        // 点击主内容区关闭移动端侧边栏
        document.addEventListener('click', (e) => {
            if (sidebar.classList.contains('open') && !sidebar.contains(e.target) && !menuToggle.contains(e.target)) {
                sidebar.classList.remove('open');
            }
        });
    }

    // 模态框关闭
    document.querySelectorAll('.modal-close').forEach(btn => {
        btn.addEventListener('click', () => {
            btn.closest('.modal').style.display = 'none';
        });
    });

    // 点击模态框外部关闭
    document.querySelectorAll('.modal').forEach(modal => {
        modal.addEventListener('click', (e) => {
            if (e.target === modal) {
                modal.style.display = 'none';
            }
        });
    });
}

// 登录处理
async function handleLogin(e) {
    e.preventDefault();
    const password = document.getElementById('login-password').value;
    try {
        const res = await fetch(`${API_BASE}/login`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ password })
        });
        if (res.ok) {
            showMainPage();
            loadDashboard();
        } else {
            const data = await res.json();
            showError('login-error', data.error?.message || '登录失败');
        }
    } catch (e) {
        showError('login-error', '网络错误');
    }
}

// 登出处理
async function handleLogout() {
    await fetch(`${API_BASE}/logout`, { method: 'POST' });
    showLoginPage();
}

// 修改密码
async function handleChangePassword(e) {
    e.preventDefault();
    const oldPassword = document.getElementById('old-password').value;
    const newPassword = document.getElementById('new-password').value;
    const confirmPassword = document.getElementById('confirm-password').value;

    if (newPassword !== confirmPassword) {
        showToast('两次输入的密码不一致', 'error');
        return;
    }

    try {
        const res = await fetch(`${API_BASE}/password`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
                old_password: oldPassword,
                new_password: newPassword
            })
        });
        const data = await res.json();
        if (res.ok) {
            showToast('密码修改成功', 'success');
            hideModal('password-modal');
            document.getElementById('password-form').reset();
        } else {
            showToast(data.error?.message || '修改失败', 'error');
        }
    } catch (e) {
        showToast('网络错误', 'error');
    }
}

// 加载仪表盘
async function loadDashboard() {
    try {
        const statsRes = await fetch(`${API_BASE}/stats`);
        const stats = await statsRes.json();

        currentEndpoints = stats.endpoints || [];
        currentPools = stats.pools || [];
        currentApis = stats.exposed_apis || [];

        // 计统计数据
        const totalErrors = currentEndpoints.reduce((sum, ep) => sum + ep.error_count, 0);
        const usageRate = stats.total_tokens_limit > 0 
            ? ((stats.total_tokens_used / stats.total_tokens_limit) * 100).toFixed(1)
            : 0;
        const usageBar = stats.total_tokens_limit > 0 
            ? Math.min((stats.total_tokens_used / stats.total_tokens_limit) * 100, 100)
            : 0;
        const usageClass = usageRate >= 100 ? 'full' : usageRate >= 80 ? 'high' : '';

        // 更新统计卡片
        document.getElementById('stat-total').textContent = stats.total_endpoints;
        document.getElementById('stat-active-sub').textContent = `活跃: ${stats.active_endpoints}`;
        document.getElementById('stat-used').textContent = formatNumber(stats.total_tokens_used);
        document.getElementById('stat-limit-sub').textContent = `限额: ${formatLimit(stats.total_tokens_limit)}`;
        document.getElementById('stat-usage-rate').textContent = `${usageRate}%`;
        document.getElementById('stat-usage-bar').style.width = `${usageBar}%`;
        document.getElementById('stat-usage-bar').className = `progress-fill ${usageClass}`;
        document.getElementById('stat-requests').textContent = formatNumber(stats.total_requests);
        document.getElementById('stat-errors-sub').textContent = `错误: ${totalErrors}`;
        document.getElementById('stat-pools').textContent = stats.total_pools;
        document.getElementById('stat-apis').textContent = stats.total_exposed_apis;
        document.getElementById('stat-total-errors').textContent = totalErrors;

        // 更新各概览区域
        renderPoolsOverview();
        renderApisOverview();
        renderEndpointsOverview();

        // 渲染图表
        renderEndpointStatusChart();
        renderTokenUsageChart();
        renderRequestChart();

        // 更新端点列表（用于端点页面）
        renderEndpointsList();
        
        // 更新池列表（用于端点页面）
        renderPoolsList();
    } catch (e) {
        console.error('加载仪表盘失败:', e);
    }
}

// 渲染端点池概览
function renderPoolsOverview() {
    const container = document.getElementById('pools-overview');
    if (!container) return;
    
    if (currentPools.length === 0) {
        container.innerHTML = '<p style="color: var(--text-tertiary); font-size: 0.875rem;">暂无端点池</p>';
        return;
    }

    const algoNames = {
        'round_robin': '轮询',
        'failover': '轮换',
        'random': '随机'
    };

    const retryNames = {
        'none': '无重试',
        'same': '原地重试',
        'pool': '端点重试'
    };

    container.innerHTML = currentPools.map(pool => `
        <div style="display: flex; justify-content: space-between; align-items: center; padding: 12px; background: var(--bg-tertiary); border-radius: var(--radius-sm); margin-bottom: 8px;">
            <div>
                <span style="font-weight: 500;">${escapeHtml(pool.name)}</span>
                <span style="font-size: 0.75rem; color: var(--text-tertiary); margin-left: 8px;">${algoNames[pool.schedule_algorithm] || pool.schedule_algorithm}</span>
            </div>
            <div style="display: flex; gap: 16px; font-size: 0.8125rem; color: var(--text-secondary);">
                <span>端点: ${pool.endpoint_count}</span>
                <span>活跃: ${pool.active_endpoint_count}</span>
                <span>Token: ${formatNumber(pool.total_tokens_used)}</span>
                <span>请求: ${formatNumber(pool.total_requests)}</span>
            </div>
        </div>
    `).join('');
}

// 渲染API接口概览
function renderApisOverview() {
    const container = document.getElementById('apis-overview');
    if (!container) return;
    
    if (currentApis.length === 0) {
        container.innerHTML = '<p style="color: var(--text-tertiary); font-size: 0.875rem;">暂无API接口</p>';
        return;
    }

    container.innerHTML = currentApis.map(api => {
        const statusClass = api.enabled ? 'active' : 'disabled';
        const statusText = api.enabled ? '启用' : '禁用';
        
        return `
            <div style="display: flex; justify-content: space-between; align-items: center; padding: 12px; background: var(--bg-tertiary); border-radius: var(--radius-sm); margin-bottom: 8px;">
                <div>
                    <span style="font-weight: 500;">${escapeHtml(api.name)}</span>
                    <span style="font-size: 0.8125rem; color: var(--accent); margin-left: 8px; font-family: var(--font-mono);">${escapeHtml(api.prefix)}</span>
                </div>
                <div style="display: flex; align-items: center; gap: 12px; font-size: 0.8125rem;">
                    <span style="color: var(--text-secondary);">${api.api_type.toUpperCase()}</span>
                    <span style="color: var(--text-secondary);">池: ${api.pool_name || '-'}</span>
                    <span style="color: var(--text-secondary);">端点: ${api.endpoint_count}</span>
                    <span class="status-badge ${statusClass}" style="font-size: 0.6875rem;">${statusText}</span>
                </div>
            </div>
        `;
    }).join('');
}

// 渲染端点概览
function renderEndpointsOverview() {
    const container = document.getElementById('endpoints-overview');
    if (currentEndpoints.length === 0) {
        container.innerHTML = '<p style="color: var(--text-secondary);">暂无端点，请在"端点管理"中添加</p>';
        return;
    }

    container.innerHTML = currentEndpoints.map(ep => {
        const percentage = ep.token_limit > 0 ? (ep.tokens_used / ep.token_limit * 100) : 0;
        const progressClass = percentage >= 100 ? 'full' : percentage >= 80 ? 'high' : '';
        const statusClass = !ep.enabled ? 'disabled' : ep.tokens_remaining === 0 ? 'exhausted' : 'active';
        const statusText = !ep.enabled ? '已禁用' : ep.tokens_remaining === 0 ? '已耗尽' : '正常';

        return `
            <div class="endpoint-card">
                <div class="endpoint-header">
                    <span class="endpoint-name">${escapeHtml(ep.name)}</span>
                    <span class="status-badge ${statusClass}">${statusText}</span>
                </div>
                <div class="endpoint-details">
                    <div class="endpoint-detail">
                        <label>类型</label>
                        <span>${ep.api_type.toUpperCase()}</span>
                    </div>
                    <div class="endpoint-detail">
                        <label>已用/限额</label>
                        <span>${formatNumber(ep.tokens_used)} / ${formatLimit(ep.token_limit)}</span>
                    </div>
                    <div class="endpoint-detail">
                        <label>请求次数限额</label>
                        <span>${ep.request_limit > 0 ? formatNumber(ep.request_limit) : '无上限'}</span>
                    </div>
                    <div class="endpoint-detail">
                        <label>已请求次数</label>
                        <span>${formatNumber(ep.requests_used)}</span>
                    </div>
                    <div class="endpoint-detail">
                        <label>总请求数</label>
                        <span>${formatNumber(ep.total_requests)}</span>
                    </div>
                    <div class="endpoint-detail">
                        <label>错误数</label>
                        <span>${ep.error_count}</span>
                    </div>
                </div>
                <div class="progress-bar">
                    <div class="progress-fill ${progressClass}" style="width: ${Math.min(percentage, 100)}%"></div>
                </div>
            </div>
        `;
    }).join('');
}

// 渲染端点状态分布图表
function renderEndpointStatusChart() {
    const container = document.getElementById('endpoint-status-chart');
    if (!container) return;

    const endpoints = currentEndpoints || [];
    const active = endpoints.filter(ep => ep.enabled && ep.error_count === 0).length;
    const error = endpoints.filter(ep => ep.enabled && ep.error_count > 0).length;
    const disabled = endpoints.filter(ep => !ep.enabled).length;
    const total = endpoints.length;

    if (total === 0) {
        container.innerHTML = '<div class="chart-empty">暂无端点数据</div>';
        return;
    }

    const max = Math.max(active, error, disabled, 1);
    const items = [
        { label: '正常', count: active, color: 'green' },
        { label: '异常', count: error, color: 'red' },
        { label: '禁用', count: disabled, color: 'gray' },
    ];

    container.innerHTML = `<div class="chart-bar-group">
        ${items.map(item => `
            <div class="chart-bar-row">
                <div class="chart-bar-label">${item.label}</div>
                <div class="chart-bar-track">
                    <div class="chart-bar-fill ${item.color}" style="width: ${(item.count / max * 100).toFixed(1)}%"></div>
                </div>
                <div class="chart-bar-value">${item.count}</div>
            </div>
        `).join('')}
    </div>`;
}

// 渲染 Token 使用量图表
function renderTokenUsageChart() {
    const container = document.getElementById('token-usage-chart');
    if (!container) return;

    const endpoints = (currentEndpoints || []).filter(ep => ep.token_limit > 0 && ep.token_limit < 999999999000 && ep.enabled);

    if (endpoints.length === 0) {
        container.innerHTML = '<div class="chart-empty">暂无有限额端点数据</div>';
        return;
    }

    const max = Math.max(...endpoints.map(ep => ep.token_limit), 1);
    const colors = ['blue', 'purple', 'cyan', 'orange', 'green', 'yellow'];

    container.innerHTML = `<div class="chart-bar-group">
        ${endpoints.map((ep, i) => {
            const percent = (ep.tokens_used / ep.token_limit * 100).toFixed(1);
            const barColor = percent >= 80 ? 'red' : percent >= 50 ? 'yellow' : colors[i % colors.length];
            return `
                <div class="chart-bar-row">
                    <div class="chart-bar-label">${escapeHtml(ep.name)}</div>
                    <div class="chart-bar-track">
                        <div class="chart-bar-fill ${barColor}" style="width: ${Math.min(percent, 100)}%"></div>
                    </div>
                    <div class="chart-bar-value">${formatNumber(ep.tokens_used)} / ${formatLimit(ep.token_limit)}</div>
                </div>
            `;
        }).join('')}
    </div>`;
}

// 渲染请求量统计图表
function renderRequestChart() {
    const container = document.getElementById('request-chart');
    if (!container) return;

    const endpoints = (currentEndpoints || []).filter(ep => ep.enabled && ep.total_requests > 0);

    if (endpoints.length === 0) {
        container.innerHTML = '<div class="chart-empty">暂无请求数据</div>';
        return;
    }

    const max = Math.max(...endpoints.map(ep => ep.total_requests), 1);
    const colors = ['blue', 'green', 'purple', 'cyan', 'orange', 'yellow'];

    container.innerHTML = `<div class="chart-bar-group">
        ${endpoints.map((ep, i) => {
            const percent = (ep.total_requests / max * 100).toFixed(1);
            return `
                <div class="chart-bar-row">
                    <div class="chart-bar-label">${escapeHtml(ep.name)}</div>
                    <div class="chart-bar-track">
                        <div class="chart-bar-fill ${colors[i % colors.length]}" style="width: ${percent}%"></div>
                    </div>
                    <div class="chart-bar-value">${formatNumber(ep.total_requests)}</div>
                </div>
            `;
        }).join('')}
    </div>`;
}

// 渲染端点列表
function renderEndpointsList() {
    const container = document.getElementById('endpoints-list');
    if (currentEndpoints.length === 0) {
        container.innerHTML = '<p style="color: var(--text-secondary);">暂无端点，点击"添加端点"开始</p>';
        return;
    }

    container.innerHTML = [...currentEndpoints]
        .sort((a, b) => a.name.localeCompare(b.name, 'zh-CN'))
        .map(ep => {
        const isUnlimited = ep.token_limit >= 999999999000;
        const percentage = (!isUnlimited && ep.token_limit > 0) ? (ep.tokens_used / ep.token_limit * 100) : 0;
        const progressClass = percentage >= 100 ? 'full' : percentage >= 80 ? 'high' : '';
        const statusClass = !ep.enabled ? 'disabled' : (!isUnlimited && ep.tokens_remaining === 0) ? 'exhausted' : 'active';
        const statusText = !ep.enabled ? '已禁用' : (!isUnlimited && ep.tokens_remaining === 0) ? '已耗尽' : '正常';

        return `
            <div class="endpoint-card">
                <div class="endpoint-header">
                    <span class="endpoint-name">${escapeHtml(ep.name)}</span>
                    <div class="endpoint-status">
                        <span class="status-badge ${statusClass}">${statusText}</span>
                    </div>
                </div>
                <div class="endpoint-details">
                    <div class="endpoint-detail">
                        <label>URL</label>
                        <span title="${escapeHtml(ep.url)}">${truncate(ep.url, 30)}</span>
                    </div>
                    <div class="endpoint-detail">
                        <label>类型</label>
                        <span>${ep.api_type.toUpperCase()}</span>
                    </div>
                    <div class="endpoint-detail">
                        <label>已使用Token</label>
                        <span>${formatNumber(ep.tokens_used)}</span>
                    </div>
                    <div class="endpoint-detail">
                        <label>Token限额</label>
                        <span>${formatLimit(ep.token_limit)}</span>
                    </div>
                    <div class="endpoint-detail">
                        <label>请求次数限额</label>
                        <span>${ep.request_limit > 0 ? formatNumber(ep.request_limit) : '无上限'}</span>
                    </div>
                    <div class="endpoint-detail">
                        <label>已请求次数</label>
                        <span>${formatNumber(ep.requests_used)}</span>
                    </div>
                </div>
                ${isUnlimited ? '' : `<div class="progress-bar">
                    <div class="progress-fill ${progressClass}" style="width: ${Math.min(percentage, 100)}%"></div>
                </div>`}
                <div class="endpoint-actions">
                    <button class="btn btn-small btn-outline" onclick="editEndpoint('${escapeAttr(ep.id)}')">编辑</button>
                    <button class="btn btn-small ${ep.enabled ? 'btn-warning' : 'btn-success'}" onclick="toggleEndpoint('${escapeAttr(ep.id)}')">
                        ${ep.enabled ? '禁用' : '启用'}
                    </button>
                    <button class="btn btn-small btn-outline" onclick="resetEndpoint('${escapeAttr(ep.id)}')">重置Token</button>
                    <button class="btn btn-small btn-outline" onclick="resetEndpointRequests('${escapeAttr(ep.id)}')">重置请求</button>
                    <button class="btn btn-small" onclick="browseEndpointModels('${escapeAttr(ep.id)}', '${escapeAttr(ep.api_type)}')">浏览模型</button>
                    <button class="btn btn-small btn-outline" onclick="quickTestEndpoint('${escapeAttr(ep.id)}', '${escapeAttr(ep.name)}')">对话测试</button>
                    <button class="btn btn-small btn-danger" onclick="deleteEndpoint('${escapeAttr(ep.id)}')">删除</button>
                </div>
            </div>
        `;
    }).join('');
}

// 端点卡片快速对话测试
async function quickTestEndpoint(id, name) {
    showToast(`正在测试 ${escapeHtml(name)}...`, 'info');
    try {
        const res = await fetch(`${API_BASE}/endpoints/${encodeURIComponent(id)}/test`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({})
        });
        const result = await res.json();
        if (result.success) {
            showToast(`${escapeHtml(name)} 测试成功: ${escapeHtml(result.message)}`, 'success');
        } else {
            showToast(`${escapeHtml(name)} 测试失败: ${escapeHtml(result.message)}`, 'error');
        }
    } catch (e) {
        showToast(`测试请求失败: ${e.message}`, 'error');
    }
}

// 添加端点到指定池
function addEndpointToPool(poolId) {
    document.getElementById('modal-title').textContent = '添加端点';
    document.getElementById('endpoint-form').reset();
    document.getElementById('ep-id').value = '';
    document.getElementById('ep-enabled').checked = true;
    
    // 清空完整路径显示
    const fullUrlDiv = document.getElementById('ep-full-url');
    if (fullUrlDiv) {
        fullUrlDiv.textContent = '';
    }
    
    // 清空测试结果
    const checkResult = document.getElementById('check-result');
    if (checkResult) {
        checkResult.style.display = 'none';
    }
    
    // 设置池ID（如果有隐藏字段）
    const poolField = document.getElementById('ep-pool-id');
    if (poolField) {
        poolField.value = poolId;
    }
    
    // 清空模型映射并更新显示
    loadModelMappings([]);
    updateModelMappingsVisibility();
    
    // 触发限额变化事件，控制重置方式（限额为空时自动设为手动重置）
    const epLimitInput = document.getElementById('ep-limit');
    if (epLimitInput) {
        epLimitInput.dispatchEvent(new Event('input'));
    }
    
    // 触发请求限制变化事件，控制重置方式
    const epReqLimitInput = document.getElementById('ep-req-limit');
    if (epReqLimitInput) {
        epReqLimitInput.dispatchEvent(new Event('input'));
    }
    
    showModal('endpoint-modal');
}

// 编辑端点
async function editEndpoint(id, fromPool = false) {
    const ep = currentEndpoints.find(e => e.id === id);
    if (!ep) return;

    // 清空上一次的测试结果/模型列表
    const checkResult = document.getElementById('check-result');
    if (checkResult) {
        checkResult.style.display = 'none';
        checkResult.innerHTML = '';
    }

    document.getElementById('modal-title').textContent = '编辑端点';
    document.getElementById('ep-id').value = ep.id;
    document.getElementById('ep-name').value = ep.name;
    document.getElementById('ep-url').value = ep.url;
    document.getElementById('ep-type').value = ep.api_type;
    document.getElementById('ep-limit').value = ep.token_limit >= 999999999000 ? '' : (ep.token_limit > 0 ? ep.token_limit : '');
    document.getElementById('ep-timeout').value = ep.timeout || 300;
    document.getElementById('ep-enabled').checked = ep.enabled;

    // 请求次数限制（0 表示无上限，显示为空）
    document.getElementById('ep-req-limit').value = ep.request_limit > 0 ? ep.request_limit : '';
    document.getElementById('ep-req-reset').value = ep.request_reset_policy || 'manual';

    // 触发请求限制变化事件，控制重置方式的禁用状态
    const epReqLimitInput = document.getElementById('ep-req-limit');
    if (epReqLimitInput) {
        epReqLimitInput.dispatchEvent(new Event('input'));
    }

    // 更新完整路径显示
    updateEndpointFullUrl();
    
    // 设置重置方式（无限制时强制为手动重置）
    const isUnlimited = ep.token_limit >= 999999999000 || ep.token_limit === 0;
    if (isUnlimited) {
        document.getElementById('ep-reset').value = 'manual';
    } else {
        document.getElementById('ep-reset').value = ep.reset_policy || 'manual';
    }
    
    // 触发限额变化事件，控制重置方式的禁用状态
    const epLimitInput = document.getElementById('ep-limit');
    if (epLimitInput) {
        epLimitInput.dispatchEvent(new Event('input'));
    }

    // 获取完整端点信息以显示 API Key 和模型映射
    try {
        const res = await fetch(`${API_BASE}/endpoints/${id}`);
        if (res.ok) {
            const fullEp = await res.json();
            document.getElementById('ep-apikey').value = fullEp.config.api_key || '';
            
            // 获取模型列表
            let models = [];
            try {
                const modelsRes = await fetch(`${API_BASE}/endpoints/models`, {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({
                        name: fullEp.config.name,
                        url: fullEp.config.url,
                        api_type: fullEp.config.api_type,
                        api_key: fullEp.config.api_key,
                        token_limit: 1000,
                        reset_policy: 'manual',
                        enabled: true
                    })
                });
                const modelsResult = await modelsRes.json();
                if (modelsResult.success && modelsResult.models) {
                    models = modelsResult.models.map(m => typeof m === 'object' ? m.id : m);
                }
            } catch (e) {
                console.error('获取模型列表失败:', e);
            }
            
            // 加载模型映射（传入模型列表）
            loadModelMappings(fullEp.config.model_mappings || [], models);
        } else {
            document.getElementById('ep-apikey').value = '';
            loadModelMappings([]);
        }
    } catch (e) {
        document.getElementById('ep-apikey').value = '';
        loadModelMappings([]);
    }

    // 设置池ID并更新模型映射显示
    document.getElementById('ep-pool-id').value = (ep.pool_ids && ep.pool_ids.length > 0) ? ep.pool_ids[0] : '';
    updateModelMappingsVisibility(fromPool);

    showModal('endpoint-modal');
}

// 浏览模型（表单内）
async function handleBrowseModelsForm() {
    const btn = document.getElementById('btn-browse-models-form');
    const checkResult = document.getElementById('check-result');
    
    const originalText = btn.textContent;
    btn.textContent = '加载中...';
    btn.disabled = true;
    
    if (checkResult) {
        checkResult.style.display = 'none';
    }

    const data = {
        name: document.getElementById('ep-name').value || 'test',
        url: document.getElementById('ep-url').value,
        api_type: document.getElementById('ep-type').value,
        api_key: document.getElementById('ep-apikey').value,
        token_limit: 1000,
        reset_policy: 'manual',
        enabled: true
    };

    if (!data.url) {
        showToast('请先填写 Base URL', 'error');
        btn.textContent = originalText;
        btn.disabled = false;
        return;
    }

    if (!data.api_key) {
        showToast('请先填写 API Key', 'error');
        btn.textContent = originalText;
        btn.disabled = false;
        return;
    }

    try {
        const res = await fetch(`${API_BASE}/endpoints/models`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(data)
        });
        const result = await res.json();
        
        if (checkResult) {
            checkResult.style.display = 'block';
            if (result.success && result.models && result.models.length > 0) {
                checkResult.style.background = 'rgba(76, 175, 80, 0.1)';
                checkResult.style.border = '1px solid rgba(76, 175, 80, 0.3)';
                
                const modelsHtml = result.models.map(m => 
                    `<div style="display: inline-block; padding: 4px 8px; margin: 4px; background: var(--bg-secondary); border-radius: 4px; font-size: 0.8125rem; font-family: var(--font-mono);">${escapeHtml(m.id)}</div>`
                ).join('');
                
                checkResult.innerHTML = `
                    <div style="color: #4caf50; font-weight: 500;">✓ 可用模型 (${result.models.length}个)</div>
                    <div style="margin-top: 8px;">${modelsHtml}</div>
                `;
            } else if (result.success) {
                checkResult.style.background = 'rgba(76, 175, 80, 0.1)';
                checkResult.style.border = '1px solid rgba(76, 175, 80, 0.3)';
                checkResult.innerHTML = `
                    <div style="color: #4caf50; font-weight: 500;">✓ 连接成功</div>
                    <div style="font-size: 0.8125rem; color: var(--text-secondary); margin-top: 4px;">未获取到模型列表</div>
                `;
            } else {
                checkResult.style.background = 'rgba(244, 67, 54, 0.1)';
                checkResult.style.border = '1px solid rgba(244, 67, 54, 0.3)';
                checkResult.innerHTML = `
                    <div style="color: #f44336; font-weight: 500;">✗ 获取失败</div>
                    <div style="font-size: 0.8125rem; color: var(--text-secondary); margin-top: 4px;">${escapeHtml(result.message)}</div>
                `;
            }
        }
        
        showToast(result.success ? '模型列表获取成功' : result.message, result.success ? 'success' : 'error');
    } catch (e) {
        showToast('请求失败: ' + e.message, 'error');
        if (checkResult) {
            checkResult.style.display = 'block';
            checkResult.style.background = 'rgba(244, 67, 54, 0.1)';
            checkResult.style.border = '1px solid rgba(244, 67, 54, 0.3)';
            checkResult.innerHTML = `
                <div style="color: #f44336; font-weight: 500;">✗ 请求失败</div>
                <div style="font-size: 0.8125rem; color: var(--text-secondary); margin-top: 4px;">${escapeHtml(e.message)}</div>
            `;
        }
    }

    btn.textContent = originalText;
    btn.disabled = false;
}

// 对话测试 - 先选择模型
async function handleCheckEndpoint() {
    const data = {
        name: document.getElementById('ep-name').value || 'test',
        url: document.getElementById('ep-url').value,
        api_type: document.getElementById('ep-type').value,
        api_key: document.getElementById('ep-apikey').value,
        token_limit: 1000,
        reset_policy: 'manual',
        enabled: true
    };

    if (!data.url) {
        showToast('请先填写 Base URL', 'error');
        return;
    }

    if (!data.api_key) {
        showToast('请先填写 API Key', 'error');
        return;
    }

    // 先获取模型列表
    const modelsList = document.getElementById('models-list');
    const modelsModalFooter = document.getElementById('models-modal-footer');
    const modelsModalTitle = document.getElementById('models-modal-title');
    
    if (modelsList) {
        modelsList.innerHTML = '<p style="color: var(--text-secondary); padding: 16px; text-align: center;">加载模型列表...</p>';
    }
    if (modelsModalFooter) {
        modelsModalFooter.style.display = 'none';
    }
    if (modelsModalTitle) {
        modelsModalTitle.textContent = '选择测试模型';
    }
    showModal('models-modal');

    try {
        const res = await fetch(`${API_BASE}/endpoints/models`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(data)
        });
        const result = await res.json();
        
        if (result.success && result.models && result.models.length > 0) {
            // 显示模型选择列表
            renderModelSelectionList(result.models, data);
            if (modelsModalFooter) {
                modelsModalFooter.style.display = 'block';
            }
        } else {
            if (modelsList) {
                modelsList.innerHTML = `<p style="color: var(--danger); padding: 16px; text-align: center;">获取模型列表失败: ${escapeHtml(result.message || '未知错误')}</p>`;
            }
        }
    } catch (e) {
        if (modelsList) {
            modelsList.innerHTML = `<p style="color: var(--danger); padding: 16px; text-align: center;">请求失败: ${escapeHtml(e.message)}</p>`;
        }
    }
}

// 渲染模型选择列表（带单选按钮）
function renderModelSelectionList(models, endpointData) {
    const container = document.getElementById('models-list');
    if (!container) return;
    
    container.innerHTML = models.map((m, index) => `
        <div style="display: flex; align-items: center; padding: 10px 12px; background: var(--bg-tertiary); border-radius: var(--radius-sm); margin-bottom: 6px; cursor: pointer;" onclick="this.querySelector('input').checked = true;">
            <input type="radio" name="selected-model" value="${escapeAttr(m.id)}" ${index === 0 ? 'checked' : ''} style="margin-right: 12px;">
            <span style="flex: 1; font-family: var(--font-mono); font-size: 0.8125rem;">${escapeHtml(m.id)}</span>
            ${m.owned_by ? `<span style="font-size: 0.75rem; color: var(--text-tertiary);">${escapeHtml(m.owned_by)}</span>` : ''}
        </div>
    `).join('');
    
    // 存储端点数据供后续使用
    container.dataset.endpointData = JSON.stringify(endpointData);
}

// 确认模型选择并进行对话测试
async function confirmModelAndTest() {
    const selectedModel = document.querySelector('input[name="selected-model"]:checked');
    if (!selectedModel) {
        showToast('请选择一个模型', 'error');
        return;
    }
    
    const container = document.getElementById('models-list');
    const endpointData = JSON.parse(container.dataset.endpointData || '{}');
    
    hideModal('models-modal');
    
    // 显示测试结果区域
    const checkResult = document.getElementById('check-result');
    const btn = document.getElementById('btn-check-endpoint');
    
    if (btn) {
        btn.textContent = '测试中...';
        btn.disabled = true;
    }
    
    if (checkResult) {
        checkResult.style.display = 'block';
        checkResult.style.background = 'rgba(33, 150, 243, 0.1)';
        checkResult.style.border = '1px solid rgba(33, 150, 243, 0.3)';
        checkResult.innerHTML = `
            <div style="color: #2196f3; font-weight: 500;">⟳ 正在测试模型: ${escapeHtml(selectedModel.value)}</div>
        `;
    }
    
    try {
        const res = await fetch(`${API_BASE}/endpoints/check`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
                ...endpointData,
                model: selectedModel.value
            })
        });
        const result = await res.json();
        
        if (checkResult) {
            if (result.success) {
                checkResult.style.background = 'rgba(76, 175, 80, 0.1)';
                checkResult.style.border = '1px solid rgba(76, 175, 80, 0.3)';
                checkResult.innerHTML = `
                    <div style="color: #4caf50; font-weight: 500;">✓ 对话测试成功</div>
                    <div style="font-size: 0.75rem; color: var(--text-tertiary); margin-top: 4px;">模型: ${escapeHtml(selectedModel.value)}</div>
                    <div style="margin-top: 8px; padding: 12px; background: var(--bg-secondary); border-radius: var(--radius-sm);">
                        <div style="font-size: 0.75rem; color: var(--text-tertiary); margin-bottom: 4px;">模型回复:</div>
                        <div style="font-size: 0.875rem; color: var(--text-primary); line-height: 1.5;">${escapeHtml(result.message)}</div>
                    </div>
                `;
            } else {
                checkResult.style.background = 'rgba(244, 67, 54, 0.1)';
                checkResult.style.border = '1px solid rgba(244, 67, 54, 0.3)';
                checkResult.innerHTML = `
                    <div style="color: #f44336; font-weight: 500;">✗ 对话测试失败</div>
                    <div style="font-size: 0.75rem; color: var(--text-tertiary); margin-top: 4px;">模型: ${escapeHtml(selectedModel.value)}</div>
                    <div style="font-size: 0.8125rem; color: var(--text-secondary); margin-top: 4px;">
                        ${result.message}
                        ${result.tested_url ? `<br>测试 URL: <code style="font-size: 0.75rem; background: var(--bg-secondary); padding: 2px 4px; border-radius: 3px;">${escapeHtml(result.tested_url)}</code>` : ''}
                    </div>
                `;
            }
        }
        
        showToast(result.success ? '对话测试成功' : result.message, result.success ? 'success' : 'error');
    } catch (e) {
        if (checkResult) {
            checkResult.style.background = 'rgba(244, 67, 54, 0.1)';
            checkResult.style.border = '1px solid rgba(244, 67, 54, 0.3)';
            checkResult.innerHTML = `
                <div style="color: #f44336; font-weight: 500;">✗ 请求失败</div>
                <div style="font-size: 0.8125rem; color: var(--text-secondary); margin-top: 4px;">${escapeHtml(e.message)}</div>
            `;
        }
        showToast('请求失败: ' + e.message, 'error');
    }
    
    if (btn) {
        btn.textContent = '对话测试';
        btn.disabled = false;
    }
}

// 保存端点
async function handleSaveEndpoint(e) {
    e.preventDefault();
    const id = document.getElementById('ep-id').value;
    const poolId = document.getElementById('ep-pool-id').value;

    // 编辑时保留端点原有的全部池归属（端点支持多池），
    // 避免表单只显示首个池导致保存后其它池里端点被静默移出。
    const originalPoolIds = id
        ? (currentEndpoints.find(e => e.id === id)?.pool_ids || [])
        : [];
    
    // 处理 token_limit：为空时默认为 12 个 9
    const limitInput = document.getElementById('ep-limit').value;
    const tokenLimit = limitInput ? parseInt(limitInput) : 999999999999;
    if (Number.isNaN(tokenLimit) || tokenLimit < 0) {
        showToast('Token 限额必须是有效数字', 'error');
        return;
    }
    
    // 处理 reset_policy：默认为每日重置
    const resetPolicy = document.getElementById('ep-reset').value || 'daily';
    
    // 处理请求次数限制
    const reqLimitInput = document.getElementById('ep-req-limit').value;
    const requestLimit = reqLimitInput ? parseInt(reqLimitInput) : 0;
    if (Number.isNaN(requestLimit) || requestLimit < 0) {
        showToast('请求次数限额必须是有效数字', 'error');
        return;
    }
    const reqResetPolicy = document.getElementById('ep-req-reset').value || 'manual';
    
    const timeout = parseInt(document.getElementById('ep-timeout').value) || 300;
    if (Number.isNaN(timeout) || timeout <= 0) {
        showToast('超时时间必须是有效正整数', 'error');
        return;
    }
    
    const data = {
        name: document.getElementById('ep-name').value,
        url: document.getElementById('ep-url').value,
        api_type: document.getElementById('ep-type').value,
        api_key: document.getElementById('ep-apikey').value,
        token_limit: tokenLimit,
        timeout: timeout,
        reset_policy: resetPolicy,
        request_limit: requestLimit,
        request_reset_policy: reqResetPolicy,
        enabled: document.getElementById('ep-enabled').checked,
        pool_ids: id ? originalPoolIds : (poolId ? [poolId] : []),
        model_mappings: getModelMappings()
    };

    // 编辑时如果api_key为空，使用原来的值；回取失败则阻止保存，避免清空已有 key
    if (id && !data.api_key) {
        const ep = currentEndpoints.find(e => e.id === id);
        if (ep) {
            try {
                const res = await fetch(`${API_BASE}/endpoints/${id}`);
                if (res.ok) {
                    const fullEp = await res.json();
                    data.api_key = fullEp.config.api_key;
                } else {
                    showToast('无法获取原 API Key，请重新填写', 'error');
                    return;
                }
            } catch (e) {
                showToast('无法获取原 API Key，请重新填写', 'error');
                return;
            }
        }
    }

    try {
        const url = id ? `${API_BASE}/endpoints/${id}` : `${API_BASE}/endpoints`;
        const method = id ? 'PUT' : 'POST';

        const res = await fetch(url, {
            method,
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(data)
        });

        if (res.ok) {
            showToast(id ? '端点已更新' : '端点已添加', 'success');
            hideModal('endpoint-modal');
            loadDashboard();
        } else {
            const err = await res.json();
            showToast(err.error?.message || '操作失败', 'error');
        }
    } catch (e) {
        showToast('网络错误', 'error');
    }
}

// 切换端点状态
async function toggleEndpoint(id) {
    try {
        const res = await fetch(`${API_BASE}/endpoints/${id}/toggle`, { method: 'POST' });
        if (res.ok) {
            showToast('端点状态已切换', 'success');
            loadDashboard();
        }
    } catch (e) {
        showToast('操作失败', 'error');
    }
}

// 重置端点Token
async function resetEndpoint(id) {
    if (!confirm('确定要重置此端点的Token使用量吗？')) return;
    try {
        const res = await fetch(`${API_BASE}/endpoints/${id}/reset`, { method: 'POST' });
        if (res.ok) {
            showToast('Token已重置', 'success');
            // 刷新当前页面数据
            const activeTab = document.querySelector('.nav-btn.active');
            if (activeTab) {
                switchTab(activeTab.dataset.tab);
            } else {
                loadDashboard();
            }
        }
    } catch (e) {
        showToast('操作失败', 'error');
    }
}

// 重置端点请求次数
async function resetEndpointRequests(id) {
    if (!confirm('确定要重置此端点的请求次数吗？')) return;
    try {
        const res = await fetch(`${API_BASE}/endpoints/${id}/reset-requests`, { method: 'POST' });
        if (res.ok) {
            showToast('请求次数已重置', 'success');
            // 刷新当前页面数据
            const activeTab = document.querySelector('.nav-btn.active');
            if (activeTab) {
                switchTab(activeTab.dataset.tab);
            } else {
                loadDashboard();
            }
        }
    } catch (e) {
        showToast('操作失败', 'error');
    }
}

// 删除端点
async function deleteEndpoint(id) {
    if (!confirm('确定要删除此端点吗？此操作不可恢复。')) return;
    try {
        const res = await fetch(`${API_BASE}/endpoints/${id}`, { method: 'DELETE' });
        if (res.ok) {
            showToast('端点已删除', 'success');
            loadDashboard();
        }
    } catch (e) {
        showToast('操作失败', 'error');
    }
}

// 重置所有
async function handleResetAll() {
    if (!confirm('确定要重置所有端点的Token使用量吗？')) return;
    try {
        const res = await fetch(`${API_BASE}/endpoints/reset-all`, { method: 'POST' });
        if (res.ok) {
            showToast('所有Token已重置', 'success');
            loadDashboard();
        }
    } catch (e) {
        showToast('操作失败', 'error');
    }
}

// 切换标签页
function switchTab(tab) {
    document.querySelectorAll('.nav-btn').forEach(btn => {
        const isActive = btn.dataset.tab === tab;
        btn.classList.toggle('active', isActive);
        btn.setAttribute('aria-selected', isActive ? 'true' : 'false');
    });
    document.querySelectorAll('.tab-content').forEach(content => {
        content.classList.toggle('active', content.id === `tab-${tab}`);
    });
    // 切换标签时加载数据
    if (tab === 'dashboard') {
        loadDashboard();
    } else if (tab === 'endpoint-mgmt') {
        loadEndpointsPage();
    } else if (tab === 'pools') {
        loadPoolsPage();
    } else if (tab === 'api-mgmt') {
        loadApisPage();
    } else if (tab === 'call-logs') {
        loadCallLogs();
    } else if (tab === 'latency-leaderboard') {
        loadLatencyLeaderboard();
    }
}

// ========== 端点管理页面 ==========

// 加载端点管理页面
async function loadEndpointsPage() {
    try {
        const statsRes = await fetch(`${API_BASE}/stats`);
        const stats = await statsRes.json();
        
        currentEndpoints = stats.endpoints || [];
        renderEndpointsList();
    } catch (e) {
        console.error('加载端点管理页面失败:', e);
    }
}

// ========== 池管理页面 ==========

// 加载池管理页面
async function loadPoolsPage() {
    try {
        const statsRes = await fetch(`${API_BASE}/stats`);
        const stats = await statsRes.json();
        
        currentEndpoints = stats.endpoints || [];
        currentPools = stats.pools || [];
        renderPoolsList();
    } catch (e) {
        console.error('加载池管理页面失败:', e);
    }
}

// ========== 选择端点到池功能 ==========

// 从池中移除端点（不删除端点，只是从当前池中移除）
async function removeEndpointFromPool(endpointId, poolId) {
    if (!confirm('确定要从池中移除此端点？移除后端点仍保留在端点管理中。')) {
        return;
    }
    
    try {
        // 先获取端点完整信息（stats API 不返回 api_key）
        const getRes = await fetch(`${API_BASE}/endpoints/${endpointId}`);
        if (!getRes.ok) {
            showToast('获取端点信息失败', 'error');
            return;
        }
        const fullEndpoint = await getRes.json();
        
        // 从 pool_ids 中移除当前池
        const currentPoolIds = (fullEndpoint.config.pool_ids || []).filter(id => id !== poolId);
        
        // 更新端点
        const res = await fetch(`${API_BASE}/endpoints/${endpointId}`, {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
                name: fullEndpoint.config.name,
                url: fullEndpoint.config.url,
                api_type: fullEndpoint.config.api_type,
                api_key: fullEndpoint.config.api_key,
                token_limit: fullEndpoint.config.token_limit,
                timeout: fullEndpoint.config.timeout || 300,
                reset_policy: fullEndpoint.config.reset_policy || 'manual',
                request_limit: fullEndpoint.config.request_limit || 0,
                request_reset_policy: fullEndpoint.config.request_reset_policy || 'manual',
                enabled: fullEndpoint.config.enabled,
                pool_ids: currentPoolIds,
                model_mappings: fullEndpoint.config.model_mappings || []
            })
        });
        
        if (res.ok) {
            showToast('已从池中移除端点', 'success');
            // 刷新池管理页面
            loadPoolsPage();
        } else {
            const data = await res.json();
            showToast(data.error?.message || '操作失败', 'error');
        }
    } catch (e) {
        console.error('从池中移除端点失败:', e);
        showToast('网络错误', 'error');
    }
}

// 显示选择端点模态框
function showSelectEndpointModal(poolId, poolName) {
    document.getElementById('select-pool-id').value = poolId;
    document.getElementById('select-endpoint-title').textContent = `选择端点到 ${poolName}`;
    
    // 检查池的模型模式
    const pool = currentPools.find(p => p.id === poolId);
    const isMappingMode = pool && pool.model_mode === 'mapping';
    
    // 获取不在当前池中的端点（支持多池，排除已在当前池的）
    const availableEndpoints = currentEndpoints.filter(ep => !(ep.pool_ids || []).includes(poolId));
    renderAvailableEndpointsList(availableEndpoints, isMappingMode);
    showModal('select-endpoint-modal');
}

// 渲染可选端点列表
function renderAvailableEndpointsList(endpoints, isMappingMode = false) {
    const container = document.getElementById('available-endpoints-list');
    if (!container) return;
    
    if (endpoints.length === 0) {
        container.innerHTML = '<p style="color: var(--text-secondary); padding: 16px; text-align: center;">没有可用的端点，请先在「端点管理」中添加端点</p>';
        return;
    }
    
    container.innerHTML = endpoints.map(ep => {
        const statusClass = !ep.enabled ? 'disabled' : ep.tokens_remaining === 0 ? 'exhausted' : 'active';
        const statusText = !ep.enabled ? '已禁用' : ep.tokens_remaining === 0 ? '已耗尽' : '正常';
        
        // 映射模式下显示模型映射配置
        const mappingHtml = isMappingMode ? `
            <div class="endpoint-mapping-config" style="margin-top: 8px; padding: 8px; background: var(--bg-secondary); border-radius: var(--radius-sm); display: none;">
                <div class="available-models" data-models="[]"></div>
                <div style="font-size: 0.75rem; color: var(--text-tertiary); margin-bottom: 8px;">配置模型映射（客户端模型名 → 端点模型名）</div>
                <div class="mapping-rows" data-endpoint-id="${ep.id}"></div>
                <button type="button" class="btn btn-small" onclick="addMappingRowInSelect('${ep.id}')" style="margin-top: 4px;">+ 添加映射</button>
            </div>
        ` : '';
        
        return `
            <div style="padding: 12px; background: var(--bg-tertiary); border-radius: var(--radius-sm); margin-bottom: 8px;">
                <div style="display: flex; align-items: center;">
                    <input type="checkbox" class="endpoint-checkbox" data-id="${ep.id}" style="margin-right: 12px;" onchange="toggleMappingConfig(this, '${ep.id}')">
                    <div style="flex: 1;">
                        <div style="display: flex; align-items: center; gap: 8px;">
                            <span style="font-weight: 500;">${escapeHtml(ep.name)}</span>
                            <span class="status-badge ${statusClass}" style="font-size: 0.625rem;">${statusText}</span>
                        </div>
                        <div style="font-size: 0.75rem; color: var(--text-secondary); margin-top: 4px;">
                            <span>${ep.api_type.toUpperCase()}</span>
                            <span style="margin-left: 8px;">${truncate(ep.url, 30)}</span>
                        </div>
                    </div>
                </div>
                ${mappingHtml}
            </div>
        `;
    }).join('');
}

// 切换模型映射配置显示
async function toggleMappingConfig(checkbox, endpointId) {
    // 找到包含 checkbox 的最外层 div
    const container = checkbox.closest('div[style*="padding"]');
    if (container) {
        const mappingConfig = container.querySelector('.endpoint-mapping-config');
        if (mappingConfig) {
            mappingConfig.style.display = checkbox.checked ? 'block' : 'none';
            
            // 勾选时加载模型列表
            if (checkbox.checked) {
                await loadEndpointModelsForSelect(endpointId, container);
            }
        }
    }
}

// 为选择端点对话框加载模型列表
async function loadEndpointModelsForSelect(endpointId, container) {
    const modelsContainer = container.querySelector('.available-models');
    if (!modelsContainer) return;
    
    try {
        // 获取端点完整信息
        const epRes = await fetch(`${API_BASE}/endpoints/${endpointId}`);
        if (!epRes.ok) return;
        const fullEp = await epRes.json();
        
        // 获取模型列表
        const res = await fetch(`${API_BASE}/endpoints/models`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
                name: fullEp.config.name,
                url: fullEp.config.url,
                api_type: fullEp.config.api_type,
                api_key: fullEp.config.api_key,
                token_limit: 1000,
                reset_policy: 'manual',
                enabled: true
            })
        });
        
        const result = await res.json();
        if (result.success && result.models && result.models.length > 0) {
            // 存储模型列表到容器（只提取 id 字段）
            const modelIds = result.models.map(m => typeof m === 'object' ? m.id : m);
            modelsContainer.dataset.models = JSON.stringify(modelIds);
        } else {
            modelsContainer.dataset.models = '[]';
        }
    } catch (e) {
        console.error('获取模型列表失败:', e);
        modelsContainer.dataset.models = '[]';
    }
}

// 在选择端点对话框中添加映射行
function addMappingRowInSelect(endpointId) {
    const container = document.querySelector(`.mapping-rows[data-endpoint-id="${endpointId}"]`);
    if (!container) return;
    
    // 获取模型列表
    const modelsContainer = container.closest('.endpoint-mapping-config')?.querySelector('.available-models');
    const models = modelsContainer?.dataset.models ? JSON.parse(modelsContainer.dataset.models) : [];
    
    // 构建模型选项
    let modelOptions = '<option value="">选择端点模型</option>';
    models.forEach(m => {
        modelOptions += `<option value="${escapeAttr(m)}">${escapeHtml(m)}</option>`;
    });
    
    const row = document.createElement('div');
    row.style.cssText = 'display: flex; gap: 8px; margin-bottom: 4px; align-items: center;';
    row.innerHTML = `
        <input type="text" class="select-mapping-client" placeholder="客户端模型名" style="flex: 1; font-size: 0.75rem;">
        <span style="color: var(--text-tertiary);">→</span>
        <select class="select-mapping-endpoint" style="flex: 1; font-size: 0.75rem;">
            ${modelOptions}
        </select>
        <button type="button" class="btn btn-small btn-danger" onclick="this.parentElement.remove()" style="font-size: 0.625rem;">删除</button>
    `;
    container.appendChild(row);
}

// 获取选择端点对话框中的模型映射
function getMappingsForEndpoint(endpointId) {
    const container = document.querySelector(`.mapping-rows[data-endpoint-id="${endpointId}"]`);
    if (!container) return [];
    
    const mappings = [];
    const rows = container.querySelectorAll('div');
    rows.forEach(row => {
        const clientModel = row.querySelector('.select-mapping-client')?.value?.trim();
        const endpointModel = row.querySelector('.select-mapping-endpoint')?.value?.trim();
        if (clientModel && endpointModel) {
            mappings.push({ client_model: clientModel, endpoint_model: endpointModel });
        }
    });
    return mappings;
}

// 搜索端点
function searchEndpointsForPool(query) {
    const poolId = document.getElementById('select-pool-id').value;
    // 获取不在当前池中的端点（支持多池）
    const availableEndpoints = currentEndpoints.filter(ep => !(ep.pool_ids || []).includes(poolId));
    
    const filtered = availableEndpoints.filter(ep => 
        ep.name.toLowerCase().includes(query.toLowerCase()) ||
        ep.url.toLowerCase().includes(query.toLowerCase())
    );
    
    // 检查池的模型模式
    const pool = currentPools.find(p => p.id === poolId);
    const isMappingMode = pool && pool.model_mode === 'mapping';
    
    renderAvailableEndpointsList(filtered, isMappingMode);
}

// 确认添加端点到池
async function confirmAddEndpointsToPool() {
    const poolId = document.getElementById('select-pool-id').value;
    const checkboxes = document.querySelectorAll('.endpoint-checkbox:checked');
    
    if (checkboxes.length === 0) {
        showToast('请选择至少一个端点', 'error');
        return;
    }
    
    const endpointIds = Array.from(checkboxes).map(cb => cb.dataset.id);
    
    // 检查池的模型模式
    const pool = currentPools.find(p => p.id === poolId);
    const isMappingMode = pool && pool.model_mode === 'mapping';
    
    // 如果是映射模式，验证是否配置了映射
    if (isMappingMode) {
        for (const endpointId of endpointIds) {
            const mappings = getMappingsForEndpoint(endpointId);
            if (mappings.length === 0) {
                showToast('映射模式下需要为每个端点配置至少一个模型映射', 'error');
                return;
            }
        }
    }
    
    try {
        // 批量更新端点的池 ID
        for (const endpointId of endpointIds) {
            // 先获取端点完整信息
            const getRes = await fetch(`${API_BASE}/endpoints/${endpointId}`);
            if (!getRes.ok) {
                throw new Error('获取端点信息失败');
            }
            const fullEndpoint = await getRes.json();
            
            // 获取该端点的模型映射配置
            const modelMappings = getMappingsForEndpoint(endpointId);
            
            // 将当前池添加到端点的 pool_ids 中
            const currentPoolIds = fullEndpoint.config.pool_ids || [];
            const newPoolIds = currentPoolIds.includes(poolId) ? currentPoolIds : [...currentPoolIds, poolId];
            
            // 更新 pool_ids
            const res = await fetch(`${API_BASE}/endpoints/${endpointId}`, {
                method: 'PUT',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({
                    name: fullEndpoint.config.name,
                    url: fullEndpoint.config.url,
                    api_type: fullEndpoint.config.api_type,
                    api_key: fullEndpoint.config.api_key,
                    token_limit: fullEndpoint.config.token_limit,
                    timeout: fullEndpoint.config.timeout || 300,
                    reset_policy: fullEndpoint.config.reset_policy || 'manual',
                    request_limit: fullEndpoint.config.request_limit || 0,
                    request_reset_policy: fullEndpoint.config.request_reset_policy || 'manual',
                    enabled: fullEndpoint.config.enabled,
                    pool_ids: newPoolIds,
                    model_mappings: modelMappings
                })
            });
            
            if (!res.ok) {
                const data = await res.json();
                throw new Error(data.error?.message || '更新失败');
            }
        }
        
        showToast(`成功添加 ${endpointIds.length} 个端点到池`, 'success');
        hideModal('select-endpoint-modal');
        
        // 刷新数据
        loadPoolsPage();
    } catch (e) {
        console.error('添加端点到池失败:', e);
        showToast('添加端点到池失败: ' + e.message, 'error');
    }
}

// ========== 模型浏览功能 ==========

// 浏览指定端点的模型列表
async function browseEndpointModels(endpointId, apiType) {
    // 从端点列表中获取端点信息
    const ep = currentEndpoints.find(e => e.id === endpointId);
    if (!ep) {
        showToast('端点不存在', 'error');
        return;
    }

    // 显示加载状态
    const modelsList = document.getElementById('models-list');
    if (modelsList) {
        modelsList.innerHTML = '<p style="color: var(--text-secondary); padding: 16px; text-align: center;">加载中...</p>';
    }
    showModal('models-modal');

    try {
        // 先获取端点完整信息（包含 api_key）
        const epRes = await fetch(`${API_BASE}/endpoints/${endpointId}`);
        if (!epRes.ok) {
            throw new Error('获取端点信息失败');
        }
        const fullEp = await epRes.json();

        // 调用浏览模型 API
        const res = await fetch(`${API_BASE}/endpoints/models`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
                name: fullEp.config.name,
                url: fullEp.config.url,
                api_type: fullEp.config.api_type,
                api_key: fullEp.config.api_key,
                token_limit: 1000,
                reset_policy: 'manual',
                enabled: true
            })
        });
        
        const result = await res.json();
        
        if (result.success && result.models && result.models.length > 0) {
            // 显示从 API 获取的真实模型列表
            renderRealModelsList(result.models);
        } else if (result.success) {
            if (modelsList) {
                modelsList.innerHTML = '<p style="color: var(--text-secondary); padding: 16px; text-align: center;">连接成功，但未获取到模型列表</p>';
            }
        } else {
            if (modelsList) {
                modelsList.innerHTML = `<p style="color: var(--danger); padding: 16px; text-align: center;">获取失败: ${escapeHtml(result.message)}</p>`;
            }
        }
    } catch (e) {
        console.error('获取模型列表失败:', e);
        if (modelsList) {
            modelsList.innerHTML = `<p style="color: var(--danger); padding: 16px; text-align: center;">请求失败: ${escapeHtml(e.message)}</p>`;
        }
    }
}

// 渲染真实模型列表
function renderRealModelsList(models) {
    const container = document.getElementById('models-list');
    if (!container) return;
    
    if (models.length === 0) {
        container.innerHTML = '<p style="color: var(--text-secondary); padding: 16px; text-align: center;">暂无可用模型</p>';
        return;
    }
    
    container.innerHTML = models.map(m => `
        <div style="display: flex; align-items: center; padding: 10px 12px; background: var(--bg-tertiary); border-radius: var(--radius-sm); margin-bottom: 6px;">
            <span style="flex: 1; font-family: var(--font-mono); font-size: 0.8125rem;">${escapeHtml(m.id)}</span>
            ${m.owned_by ? `<span style="font-size: 0.75rem; color: var(--text-tertiary);">${escapeHtml(m.owned_by)}</span>` : ''}
        </div>
    `).join('');
}

// 浏览模型列表（显示所有模型）
async function browseModels() {
    try {
        // 获取当前端点的 API 类型来决定显示哪些模型
        const openaiEndpoints = currentEndpoints.filter(ep => ep.api_type === 'openai' || ep.api_type === 'openai-responses');
        const anthropicEndpoints = currentEndpoints.filter(ep => ep.api_type === 'anthropic');
        
        // 常见模型列表
        const models = {
            openai: [
                { id: 'gpt-4o', name: 'GPT-4o', description: '最新旗舰模型，支持多模态' },
                { id: 'gpt-4o-mini', name: 'GPT-4o Mini', description: '性价比最高的小型模型' },
                { id: 'gpt-4-turbo', name: 'GPT-4 Turbo', description: 'GPT-4 Turbo with vision' },
                { id: 'gpt-4', name: 'GPT-4', description: '强大的推理能力' },
                { id: 'gpt-3.5-turbo', name: 'GPT-3.5 Turbo', description: '快速且经济实惠' },
                { id: 'o1-preview', name: 'o1-preview', description: '推理模型预览版' },
                { id: 'o1-mini', name: 'o1-mini', description: '小型推理模型' },
            ],
            anthropic: [
                { id: 'claude-3-5-sonnet-20241022', name: 'Claude 3.5 Sonnet', description: '最新旗舰模型' },
                { id: 'claude-3-5-haiku-20241022', name: 'Claude 3.5 Haiku', description: '快速轻量模型' },
                { id: 'claude-3-opus-20240229', name: 'Claude 3 Opus', description: '最强大的推理能力' },
                { id: 'claude-3-sonnet-20240229', name: 'Claude 3 Sonnet', description: '平衡性能与速度' },
                { id: 'claude-3-haiku-20240307', name: 'Claude 3 Haiku', description: '最快速的响应' },
            ]
        };
        
        let allModels = [];
        if (openaiEndpoints.length > 0) {
            allModels = allModels.concat(models.openai.map(m => ({ ...m, type: 'OpenAI' })));
        }
        if (anthropicEndpoints.length > 0) {
            allModels = allModels.concat(models.anthropic.map(m => ({ ...m, type: 'Anthropic' })));
        }
        
        // 如果没有端点，显示所有模型
        if (allModels.length === 0) {
            allModels = [
                ...models.openai.map(m => ({ ...m, type: 'OpenAI' })),
                ...models.anthropic.map(m => ({ ...m, type: 'Anthropic' }))
            ];
        }
        
        renderModelsList(allModels);
        showModal('models-modal');
    } catch (e) {
        console.error('获取模型列表失败:', e);
        showToast('获取模型列表失败', 'error');
    }
}

// 渲染模型列表
function renderModelsList(models) {
    const container = document.getElementById('models-list');
    if (!container) return;
    
    if (models.length === 0) {
        container.innerHTML = '<p style="color: var(--text-secondary); padding: 16px; text-align: center;">暂无可用模型</p>';
        return;
    }
    
    container.innerHTML = models.map(model => `
        <div style="display: flex; justify-content: space-between; align-items: center; padding: 12px; background: var(--bg-tertiary); border-radius: var(--radius-sm); margin-bottom: 8px;">
            <div>
                <span style="font-weight: 500;">${escapeHtml(model.name)}</span>
                <span style="font-size: 0.75rem; color: var(--text-tertiary); margin-left: 8px;">${model.type}</span>
                <p style="font-size: 0.8125rem; color: var(--text-secondary); margin-top: 4px;">${escapeHtml(model.description)}</p>
            </div>
            <div style="display: flex; align-items: center; gap: 8px;">
                <code style="font-size: 0.75rem; background: var(--bg-secondary); padding: 2px 6px; border-radius: 4px;">${escapeHtml(model.id)}</code>
            </div>
        </div>
    `).join('');
}

// 搜索模型
function searchModels(query) {
    const allModels = [
        { id: 'gpt-4o', name: 'GPT-4o', description: '最新旗舰模型，支持多模态', type: 'OpenAI' },
        { id: 'gpt-4o-mini', name: 'GPT-4o Mini', description: '性价比最高的小型模型', type: 'OpenAI' },
        { id: 'gpt-4-turbo', name: 'GPT-4 Turbo', description: 'GPT-4 Turbo with vision', type: 'OpenAI' },
        { id: 'gpt-4', name: 'GPT-4', description: '强大的推理能力', type: 'OpenAI' },
        { id: 'gpt-3.5-turbo', name: 'GPT-3.5 Turbo', description: '快速且经济实惠', type: 'OpenAI' },
        { id: 'o1-preview', name: 'o1-preview', description: '推理模型预览版', type: 'OpenAI' },
        { id: 'o1-mini', name: 'o1-mini', description: '小型推理模型', type: 'OpenAI' },
        { id: 'claude-3-5-sonnet-20241022', name: 'Claude 3.5 Sonnet', description: '最新旗舰模型', type: 'Anthropic' },
        { id: 'claude-3-5-haiku-20241022', name: 'Claude 3.5 Haiku', description: '快速轻量模型', type: 'Anthropic' },
        { id: 'claude-3-opus-20240229', name: 'Claude 3 Opus', description: '最强大的推理能力', type: 'Anthropic' },
        { id: 'claude-3-sonnet-20240229', name: 'Claude 3 Sonnet', description: '平衡性能与速度', type: 'Anthropic' },
        { id: 'claude-3-haiku-20240307', name: 'Claude 3 Haiku', description: '最快速的响应', type: 'Anthropic' },
    ];
    
    const filtered = allModels.filter(model => 
        model.id.toLowerCase().includes(query.toLowerCase()) ||
        model.name.toLowerCase().includes(query.toLowerCase()) ||
        model.description.toLowerCase().includes(query.toLowerCase())
    );
    
    renderModelsList(filtered);
}

// 显示/隐藏页面
function showLoginPage() {
    document.getElementById('login-page').style.display = 'flex';
    document.getElementById('main-page').style.display = 'none';
}

function showMainPage() {
    document.getElementById('login-page').style.display = 'none';
    document.getElementById('main-page').style.display = 'block';
}

// 模态框
function showModal(id) {
    const modal = document.getElementById(id);
    modal.style.display = 'flex';
    
    // 点击遮罩层关闭模态框
    modal.onclick = function(e) {
        if (e.target === modal) {
            hideModal(id);
        }
    };
    
    // ESC 键关闭模态框
    const escHandler = function(e) {
        if (e.key === 'Escape') {
            hideModal(id);
            document.removeEventListener('keydown', escHandler);
        }
    };
    document.addEventListener('keydown', escHandler);
    
    // 聚焦到第一个输入框
    setTimeout(() => {
        const firstInput = modal.querySelector('input:not([type="hidden"]):not([type="checkbox"]), select');
        if (firstInput) firstInput.focus();
    }, 100);
}

function hideModal(id) {
    document.getElementById(id).style.display = 'none';
}

// ========== 对外API和池管理 ==========

// 加载对外接口页面
async function loadApisPage() {
    try {
        const [statsRes] = await Promise.all([
            fetch(`${API_BASE}/stats`)
        ]);
        const stats = await statsRes.json();
        
        currentPools = stats.pools || [];
        currentApis = stats.exposed_apis || [];
        
        renderApisList();
        renderPoolsList();
    } catch (e) {
        console.error('加载对外接口页面失败:', e);
    }
}

// 渲染对外API列表
function renderApisList() {
    const container = document.getElementById('apis-list');
    if (currentApis.length === 0) {
        container.innerHTML = '<p style="color: var(--text-tertiary); font-size: 0.875rem;">暂无对外接口，点击"添加接口"开始配置</p>';
        return;
    }

    const baseUrl = window.location.origin;

    container.innerHTML = currentApis.map(api => {
        const statusClass = api.enabled ? 'active' : 'disabled';
        const statusText = api.enabled ? '已启用' : '已禁用';
        
        // 构建完整调用 URL
        let examplePath = '';
        switch (api.api_type) {
            case 'openai':
                examplePath = '/chat/completions';
                break;
            case 'anthropic':
                examplePath = '/messages';
                break;
            case 'openai-responses':
                examplePath = '/responses';
                break;
            default:
                examplePath = '/chat/completions';
        }
        const fullCallUrl = `${baseUrl}${api.prefix}${examplePath}`;
        
        return `
            <div class="endpoint-card">
                <div class="endpoint-header">
                    <span class="endpoint-name">${escapeHtml(api.name)}</span>
                    <div class="endpoint-status">
                        <span class="status-badge ${statusClass}">${statusText}</span>
                    </div>
                </div>
                <div class="endpoint-details">
                    <div class="endpoint-detail">
                        <label>前缀</label>
                        <span style="color: var(--accent);">${escapeHtml(api.prefix)}</span>
                    </div>
                    <div class="endpoint-detail">
                        <label>类型</label>
                        <span>${api.api_type.toUpperCase()}</span>
                    </div>
                    <div class="endpoint-detail">
                        <label>关联池</label>
                        <span>${api.pool_name || '未关联'}</span>
                    </div>
                    <div class="endpoint-detail">
                        <label>端点数</label>
                        <span>${api.endpoint_count}</span>
                    </div>
                </div>
                <div style="margin-top: 8px; padding: 8px 12px; background: var(--bg-secondary); border-radius: var(--radius-sm); font-family: var(--font-mono); font-size: 0.75rem; color: var(--text-secondary); word-break: break-all;">
                    调用URL: ${escapeHtml(fullCallUrl)}
                </div>
                <div class="endpoint-actions">
                    <button class="btn btn-small" onclick="editApi('${escapeAttr(api.id)}')">编辑</button>
                    <button class="btn btn-small ${api.enabled ? 'btn-danger' : ''}" onclick="toggleApi('${escapeAttr(api.id)}')">
                        ${api.enabled ? '禁用' : '启用'}
                    </button>
                    <button class="btn btn-small btn-danger" onclick="deleteApi('${escapeAttr(api.id)}')">删除</button>
                </div>
            </div>
        `;
    }).join('');
}

// 渲染池列表（包含池内的端点）
function renderPoolsList() {
    const container = document.getElementById('pools-list');
    if (currentPools.length === 0) {
        container.innerHTML = '<p style="color: var(--text-tertiary); font-size: 0.875rem;">暂无端点池，点击"添加池"开始配置</p>';
        return;
    }

    const algoNames = {
        'round_robin': '轮询',
        'failover': '轮换',
        'random': '随机'
    };

    const retryNames = {
        'none': '无重试',
        'same': '原地重试',
        'pool': '端点重试'
    };

    container.innerHTML = currentPools.map(pool => {
        // 获取该池下的端点
        const poolEndpoints = currentEndpoints.filter(ep => (ep.pool_ids || []).includes(pool.id));
        
        const endpointsHtml = poolEndpoints.length > 0 ? poolEndpoints.map(ep => {
            const statusClass = !ep.enabled ? 'disabled' : ep.tokens_remaining === 0 ? 'exhausted' : 'active';
            const statusText = !ep.enabled ? '已禁用' : ep.tokens_remaining === 0 ? '已耗尽' : '正常';
            const percentage = ep.token_limit > 0 ? (ep.tokens_used / ep.token_limit * 100) : 0;
            const progressClass = percentage >= 100 ? 'full' : percentage >= 80 ? 'high' : '';

            return `
                <div style="padding: 12px; background: var(--bg-primary); border-radius: var(--radius-sm); margin-top: 8px; border-left: 3px solid var(--accent);">
                    <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 8px;">
                        <span style="font-weight: 500; font-size: 0.875rem;">${escapeHtml(ep.name)}</span>
                        <div style="display: flex; align-items: center; gap: 8px;">
                            <span class="status-badge ${statusClass}" style="font-size: 0.625rem;">${statusText}</span>
                        </div>
                    </div>
                    <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 4px; font-size: 0.75rem; color: var(--text-secondary); margin-bottom: 8px;">
                        <span>URL: ${truncate(ep.url, 25)}</span>
                        <span>类型: ${ep.api_type.toUpperCase()}</span>
                        <span>已用: ${formatNumber(ep.tokens_used)} / ${formatLimit(ep.token_limit)}</span>
                        <span>请求: ${ep.total_requests}</span>
                    </div>
                    <div class="progress-bar" style="height: 3px; margin-bottom: 8px;">
                        <div class="progress-fill ${progressClass}" style="width: ${Math.min(percentage, 100)}%"></div>
                    </div>
                    <div style="display: flex; gap: 6px;">
                        <button class="btn btn-small" onclick="editEndpoint('${escapeAttr(ep.id)}', true)" style="font-size: 0.6875rem;">编辑</button>
                        <button class="btn btn-small btn-warning" onclick="removeEndpointFromPool('${escapeAttr(ep.id)}', '${escapeAttr(pool.id)}')" style="font-size: 0.6875rem;">从池中移除</button>
                    </div>
                </div>
            `;
        }).join('') : '<p style="font-size: 0.75rem; color: var(--text-tertiary); margin-top: 8px; padding: 8px;">暂无端点，点击下方按钮添加</p>';

        return `
            <div class="endpoint-card" style="margin-bottom: 16px;">
                <div class="endpoint-header">
                    <span class="endpoint-name">${escapeHtml(pool.name)}</span>
                    <span class="status-badge active">${algoNames[pool.schedule_algorithm] || pool.schedule_algorithm}</span>
                    ${pool.retry_mode && pool.retry_mode !== 'none' ? `<span class="status-badge" style="background: rgba(255,152,0,0.1); color: #ff9800;">${retryNames[pool.retry_mode] || pool.retry_mode} ${pool.retry_count}次</span>` : ''}
                </div>
                <div class="endpoint-details">
                    <div class="endpoint-detail">
                        <label>描述</label>
                        <span>${escapeHtml(pool.description || '无')}</span>
                    </div>
                    <div class="endpoint-detail">
                        <label>端点数</label>
                        <span>${pool.endpoint_count}</span>
                    </div>
                    <div class="endpoint-detail">
                        <label>活跃</label>
                        <span>${pool.active_endpoint_count}</span>
                    </div>
                    <div class="endpoint-detail">
                        <label>已用Token</label>
                        <span>${formatNumber(pool.total_tokens_used)}</span>
                    </div>
                    <div class="endpoint-detail">
                        <label>请求数</label>
                        <span>${formatNumber(pool.total_requests)}</span>
                    </div>
                </div>
                
                <!-- 池内端点列表 -->
                <div style="margin-top: 12px; padding-top: 12px; border-top: 1px solid var(--border);">
                    <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 8px;">
                        <span style="font-size: 0.8125rem; font-weight: 500; color: var(--text-secondary);">池内端点</span>
                        <button class="btn btn-small" onclick="showSelectEndpointModal('${escapeAttr(pool.id)}', '${escapeAttr(pool.name)}')" style="font-size: 0.6875rem;">选择端点</button>
                    </div>
                    ${endpointsHtml}
                </div>
                
                <div class="endpoint-actions" style="margin-top: 12px;">
                    <button class="btn btn-small" onclick="handlePoolTest('${escapeAttr(pool.id)}', '${escapeAttr(pool.name)}')" style="background: var(--accent); color: #fff;">一键测试</button>
                    <button class="btn btn-small" onclick="editPool('${escapeAttr(pool.id)}')">编辑池</button>
                    <button class="btn btn-small btn-danger" onclick="deletePool('${escapeAttr(pool.id)}')">删除池</button>
                </div>
            </div>
        `;
    }).join('');
}

// 池一键测试
let currentPoolTestId = null;
let currentPoolTestName = null;
let currentPoolTestModels = [];

async function handlePoolTest(poolId, poolName) {
    currentPoolTestId = poolId;
    currentPoolTestName = poolName;

    const title = document.getElementById('pool-test-title');
    title.textContent = `池测试: ${poolName}`;

    // 重置到配置阶段
    showConfigPhase();

    // 加载模型列表
    const modelsList = document.getElementById('pool-test-models-list');
    modelsList.innerHTML = '<p style="color: var(--text-secondary); padding: 16px; text-align: center;">加载模型列表...</p>';

    showModal('pool-test-modal');

    try {
        const res = await fetch(`${API_BASE}/pools/${poolId}/models`);
        const data = await res.json();
        if (data.success && data.models) {
            currentPoolTestModels = data.models;
            renderPoolTestModelList(data.models);
        } else {
            modelsList.innerHTML = '<p style="color: var(--text-tertiary); padding: 16px; text-align: center;">无法获取模型列表，将使用自动模式</p>';
        }
    } catch (e) {
        modelsList.innerHTML = `<p style="color: #f44336; padding: 16px;">加载失败: ${escapeHtml(e.message)}</p>`;
    }
}

function showConfigPhase() {
    document.getElementById('pool-test-config').style.display = 'block';
    document.getElementById('pool-test-results-area').style.display = 'none';
    document.querySelector('input[name="pool-test-mode"][value="auto"]').checked = true;
    document.getElementById('pool-test-model-select').style.display = 'none';
}

function onPoolTestModeChange() {
    const mode = document.querySelector('input[name="pool-test-mode"]:checked').value;
    document.getElementById('pool-test-model-select').style.display = mode === 'manual' ? 'block' : 'none';
    if (mode === 'manual' && currentPoolTestModels.length > 0) {
        renderPoolTestModelList(currentPoolTestModels);
    }
}

function renderPoolTestModelList(models) {
    const container = document.getElementById('pool-test-models-list');
    const searchInput = document.getElementById('pool-test-model-search');
    searchInput.value = '';

    const render = (filter) => {
        const filtered = filter ? models.filter(m => m.toLowerCase().includes(filter.toLowerCase())) : models;
        container.innerHTML = filtered.map((m, i) => `
            <div style="display: flex; align-items: center; padding: 10px 12px; background: var(--bg-tertiary); border-bottom: 1px solid var(--border); cursor: pointer;"
                 onclick="selectPoolTestModel('${escapeAttr(m)}', this)">
                <input type="radio" name="pool-test-selected-model" value="${escapeAttr(m)}" ${i === 0 ? 'checked' : ''} style="margin-right: 12px;">
                <span style="flex: 1; font-family: var(--font-mono); font-size: 0.8125rem;">${escapeHtml(m)}</span>
            </div>
        `).join('');
    };

    render(null);

    searchInput.oninput = () => render(searchInput.value);
}

function selectPoolTestModel(modelName, el) {
    el.querySelector('input').checked = true;
}

async function startPoolTest() {
    const mode = document.querySelector('input[name="pool-test-mode"]:checked').value;
    let selectedModel = null;

    if (mode === 'manual') {
        const checked = document.querySelector('input[name="pool-test-selected-model"]:checked');
        if (!checked) {
            showToast('请选择一个模型', 'error');
            return;
        }
        selectedModel = checked.value;
    }

    // 切换到结果阶段
    document.getElementById('pool-test-config').style.display = 'none';
    document.getElementById('pool-test-results-area').style.display = 'block';

    const summaryEl = document.getElementById('pool-test-summary');
    const resultsEl = document.getElementById('pool-test-results');
    summaryEl.style.background = 'rgba(33, 150, 243, 0.1)';
    summaryEl.style.border = '1px solid rgba(33, 150, 243, 0.3)';
    summaryEl.innerHTML = '<span style="color: #2196f3; font-weight: 500;">正在测试所有端点...</span>';
    resultsEl.innerHTML = '<p style="color: var(--text-secondary); text-align: center; padding: 32px;">正在发送测试请求，请稍候...</p>';

    try {
        const body = { mode: mode };
        if (selectedModel) body.model = selectedModel;

        const res = await fetch(`${API_BASE}/pools/${currentPoolTestId}/test-all`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(body)
        });
        const data = await res.json();

        if (!data.results || data.results.length === 0) {
            summaryEl.style.background = 'rgba(255, 152, 0, 0.1)';
            summaryEl.style.border = '1px solid rgba(255, 152, 0, 0.3)';
            summaryEl.innerHTML = '<span style="color: #ff9800; font-weight: 500;">该池下没有可测试的端点</span>';
            resultsEl.innerHTML = '<p style="color: var(--text-tertiary); text-align: center; padding: 16px;">请先添加端点到该池</p>';
            return;
        }

        const summary = data.summary;
        const allOk = summary.failed === 0;
        summaryEl.style.background = allOk ? 'rgba(76, 175, 80, 0.1)' : 'rgba(244, 67, 54, 0.1)';
        summaryEl.style.border = allOk ? '1px solid rgba(76, 175, 80, 0.3)' : '1px solid rgba(244, 67, 54, 0.3)';
        summaryEl.innerHTML = `
            <div style="display: flex; gap: 24px; align-items: center;">
                <span style="font-weight: 600; color: ${allOk ? '#4caf50' : '#f44336'};">
                    ${allOk ? '全部通过' : '部分失败'}
                </span>
                <span style="font-size: 0.875rem; color: var(--text-secondary);">总计: <strong>${summary.total}</strong></span>
                <span style="font-size: 0.875rem; color: #4caf50;">成功: <strong>${summary.success}</strong></span>
                <span style="font-size: 0.875rem; color: #f44336;">失败: <strong>${summary.failed}</strong></span>
            </div>
        `;

        resultsEl.innerHTML = data.results.map(r => {
            const statusColor = r.success ? '#4caf50' : '#f44336';
            const borderColor = r.success ? 'var(--accent)' : '#f44336';
            const statusText = r.success ? '成功' : '失败';
            const statusClass = r.success ? 'active' : 'disabled';

            return `
                <div style="padding: 12px; background: var(--bg-primary); border-radius: var(--radius-sm); margin-bottom: 8px; border-left: 3px solid ${borderColor};">
                    <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 8px;">
                        <span style="font-weight: 500; font-size: 0.875rem;">${escapeHtml(r.endpoint_name)}</span>
                        <span class="status-badge ${statusClass}" style="font-size: 0.625rem; background: ${statusColor}20; color: ${statusColor};">${statusText}</span>
                    </div>
                    <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 4px; font-size: 0.75rem; color: var(--text-secondary); margin-bottom: 8px;">
                        <span>模型: ${escapeHtml(r.model_used)}</span>
                        <span>HTTP ${r.status}</span>
                    </div>
                    <div style="font-size: 0.8125rem; color: ${r.success ? 'var(--text-primary)' : '#f44336'}; padding: 6px 10px; background: var(--bg-tertiary); border-radius: var(--radius-sm); word-break: break-all;">
                        ${escapeHtml(r.message)}
                    </div>
                </div>
            `;
        }).join('');
    } catch (e) {
        summaryEl.style.background = 'rgba(244, 67, 54, 0.1)';
        summaryEl.style.border = '1px solid rgba(244, 67, 54, 0.3)';
        summaryEl.innerHTML = '<span style="color: #f44336; font-weight: 500;">测试请求失败</span>';
        resultsEl.innerHTML = `<p style="color: #f44336; padding: 16px;">${escapeHtml(e.message)}</p>`;
    }
}

// ========== 端点映射配置 ==========

// 显示端点映射配置对话框
async function showEndpointMappingModal(endpointId) {
    const ep = currentEndpoints.find(e => e.id === endpointId);
    if (!ep) return;
    
    document.getElementById('mapping-endpoint-id').value = endpointId;
    document.getElementById('mapping-endpoint-name').textContent = ep.name;
    
    // 获取端点完整信息（包含模型映射）
    try {
        const res = await fetch(`${API_BASE}/endpoints/${endpointId}`);
        if (res.ok) {
            const fullEp = await res.json();
            const mappings = fullEp.config.model_mappings || [];
            
            // 获取模型列表
            const modelsRes = await fetch(`${API_BASE}/endpoints/models`, {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({
                    name: fullEp.config.name,
                    url: fullEp.config.url,
                    api_type: fullEp.config.api_type,
                    api_key: fullEp.config.api_key,
                    token_limit: 1000,
                    reset_policy: 'manual',
                    enabled: true
                })
            });
            const modelsResult = await modelsRes.json();
            const models = modelsResult.success ? (modelsResult.models || []).map(m => typeof m === 'object' ? m.id : m) : [];
            
            // 渲染映射列表
            renderEndpointMappingList(mappings, models);
        }
    } catch (e) {
        console.error('获取端点信息失败:', e);
    }
    
    showModal('endpoint-mapping-modal');
}

// 渲染端点映射列表
function renderEndpointMappingList(mappings, models) {
    const container = document.getElementById('endpoint-mapping-list');
    if (!container) return;
    
    // 存储模型列表
    container.dataset.models = JSON.stringify(models);
    
    container.innerHTML = '';
    if (mappings.length > 0) {
        mappings.forEach(m => addEndpointMappingRowWithData(m.client_model, m.endpoint_model, models));
    }
}

// 添加端点映射行
function addEndpointMappingRow() {
    const container = document.getElementById('endpoint-mapping-list');
    const models = container.dataset.models ? JSON.parse(container.dataset.models) : [];
    addEndpointMappingRowWithData('', '', models);
}

// 添加端点映射行（带数据）
function addEndpointMappingRowWithData(clientModel, endpointModel, models) {
    const container = document.getElementById('endpoint-mapping-list');
    if (!container) return;
    
    let modelOptions = '<option value="">选择端点模型</option>';
    models.forEach(m => {
        const selected = m === endpointModel ? 'selected' : '';
        modelOptions += `<option value="${escapeAttr(m)}" ${selected}>${escapeHtml(m)}</option>`;
    });
    
    const row = document.createElement('div');
    row.style.cssText = 'display: flex; gap: 8px; margin-bottom: 8px; align-items: center;';
    row.innerHTML = `
        <input type="text" class="ep-mapping-client" placeholder="客户端模型名" value="${escapeAttr(clientModel)}" style="flex: 1;">
        <span style="color: var(--text-tertiary);">→</span>
        <select class="ep-mapping-endpoint" style="flex: 1;">
            ${modelOptions}
        </select>
        <button type="button" class="btn btn-small btn-danger" onclick="this.parentElement.remove()">删除</button>
    `;
    container.appendChild(row);
}

// 保存端点映射
async function saveEndpointMapping() {
    const endpointId = document.getElementById('mapping-endpoint-id').value;
    const container = document.getElementById('endpoint-mapping-list');
    
    // 收集映射数据
    const mappings = [];
    const rows = container.querySelectorAll('div');
    rows.forEach(row => {
        const clientModel = row.querySelector('.ep-mapping-client')?.value?.trim();
        const endpointModel = row.querySelector('.ep-mapping-endpoint')?.value;
        if (clientModel && endpointModel) {
            mappings.push({ client_model: clientModel, endpoint_model: endpointModel });
        }
    });
    
    // 获取端点完整信息
    try {
        const getRes = await fetch(`${API_BASE}/endpoints/${endpointId}`);
        if (!getRes.ok) {
            showToast('获取端点信息失败', 'error');
            return;
        }
        const fullEp = await getRes.json();
        
        // 更新端点映射
        const res = await fetch(`${API_BASE}/endpoints/${endpointId}`, {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
                name: fullEp.config.name,
                url: fullEp.config.url,
                api_type: fullEp.config.api_type,
                api_key: fullEp.config.api_key,
                token_limit: fullEp.config.token_limit,
                timeout: fullEp.config.timeout || 300,
                reset_policy: fullEp.config.reset_policy || 'manual',
                request_limit: fullEp.config.request_limit || 0,
                request_reset_policy: fullEp.config.request_reset_policy || 'manual',
                enabled: fullEp.config.enabled,
                pool_ids: fullEp.config.pool_ids || [],
                model_mappings: mappings
            })
        });
        
        if (res.ok) {
            showToast('模型映射已保存', 'success');
            hideModal('endpoint-mapping-modal');
            // 刷新数据
            loadPoolsPage();
        } else {
            const data = await res.json();
            showToast(data.error?.message || '保存失败', 'error');
        }
    } catch (e) {
        showToast('保存失败: ' + e.message, 'error');
    }
}

// 加载池选项到下拉框
async function loadPoolOptions(selectId) {
    try {
        const res = await fetch(`${API_BASE}/stats`);
        const stats = await res.json();
        currentPools = stats.pools || [];
        
        const select = document.getElementById(selectId);
        select.innerHTML = '<option value="">请选择池</option>' + 
            currentPools.map(p => `<option value="${p.id}">${escapeHtml(p.name)}</option>`).join('');
    } catch (e) {
        console.error('加载池选项失败:', e);
    }
}

// 编辑对外API
async function editApi(id) {
    const api = currentApis.find(a => a.id === id);
    if (!api) return;
    
    document.getElementById('api-modal-title').textContent = '编辑对外接口';
    document.getElementById('api-id').value = api.id;
    document.getElementById('api-name').value = api.name;
    document.getElementById('api-prefix').value = api.prefix;
    document.getElementById('api-type').value = api.api_type;
    document.getElementById('api-enabled').checked = api.enabled;
    
    await loadPoolOptions('api-pool');
    document.getElementById('api-pool').value = api.pool_id;

    // 获取完整接口信息（stats 接口不返回 api_key），正确填充认证密钥
    try {
        const res = await fetch(`${API_BASE}/exposed-apis/${api.id}`);
        if (res.ok) {
            const fullApi = await res.json();
            document.getElementById('api-key').value = fullApi.api_key || '';
        } else {
            document.getElementById('api-key').value = '';
        }
    } catch (e) {
        console.error('加载接口详情失败:', e);
        document.getElementById('api-key').value = '';
    }

    // 更新完整 URL 显示
    updateApiFullUrlDisplay();
    
    // 清空测试结果
    const apiTestResult = document.getElementById('api-test-result');
    if (apiTestResult) {
        apiTestResult.style.display = 'none';
    }
    
    showModal('api-modal');
}

// 对外接口对话测试 - 先选择模型
async function handleTestApi() {
    const prefix = document.getElementById('api-prefix').value.trim();
    const apiKey = document.getElementById('api-key').value;
    const apiType = document.getElementById('api-type').value;

    if (!prefix) {
        showToast('请先填写 URL 前缀', 'error');
        return;
    }

    // 构建测试 URL
    const baseUrl = window.location.origin;
    const cleanPrefix = prefix.startsWith('/') ? prefix : '/' + prefix;
    
    // 获取关联的端点池信息
    const poolId = document.getElementById('api-pool').value;
    if (!poolId) {
        showToast('请先选择关联端点池', 'error');
        return;
    }

    // 先获取模型列表
    const modelsList = document.getElementById('models-list');
    const modelsModalFooter = document.getElementById('models-modal-footer');
    const modelsModalTitle = document.getElementById('models-modal-title');
    
    if (modelsList) {
        modelsList.innerHTML = '<p style="color: var(--text-secondary); padding: 16px; text-align: center;">加载模型列表...</p>';
    }
    if (modelsModalFooter) {
        modelsModalFooter.style.display = 'none';
    }
    if (modelsModalTitle) {
        modelsModalTitle.textContent = '选择测试模型';
    }
    showModal('models-modal');

    // 获取池中的端点信息来调用模型列表
    try {
        const statsRes = await fetch(`${API_BASE}/stats`);
        const stats = await statsRes.json();
        const poolEndpoints = (stats.endpoints || []).filter(ep => (ep.pool_ids || []).includes(poolId));
        const pool = (stats.pools || []).find(p => p.id === poolId);
        
        if (poolEndpoints.length === 0) {
            if (modelsList) {
                modelsList.innerHTML = '<p style="color: var(--danger); padding: 16px; text-align: center;">关联池中没有端点，请先添加端点</p>';
            }
            return;
        }

        let models = [];
        let modelMappings = []; // 存储映射关系
        let selectedEndpointId = null; // 记录选择的端点ID
        
        // 随机选择一个端点
        const randomIndex = Math.floor(Math.random() * poolEndpoints.length);
        const selectedEndpoint = poolEndpoints[randomIndex];
        selectedEndpointId = selectedEndpoint.id;
        
        // 映射模式：从选中端点的模型映射中获取客户端模型名称
        if (pool && pool.model_mode === 'mapping') {
            const epRes = await fetch(`${API_BASE}/endpoints/${selectedEndpoint.id}`);
            if (epRes.ok) {
                const fullEp = await epRes.json();
                const mappings = fullEp.config.model_mappings || [];
                models = mappings.map(m => m.client_model);
                modelMappings = mappings;
            }
        } else {
            // 透传模式：从选中端点获取模型列表
            const epRes = await fetch(`${API_BASE}/endpoints/${selectedEndpoint.id}`);
            if (!epRes.ok) {
                throw new Error('获取端点信息失败');
            }
            const fullEp = await epRes.json();

            const modelsRes = await fetch(`${API_BASE}/endpoints/models`, {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({
                    name: fullEp.config.name,
                    url: fullEp.config.url,
                    api_type: fullEp.config.api_type,
                    api_key: fullEp.config.api_key,
                    token_limit: 1000,
                    reset_policy: 'manual',
                    enabled: true
                })
            });
            const modelsResult = await modelsRes.json();
            if (modelsResult.success && modelsResult.models) {
                models = modelsResult.models.map(m => typeof m === 'object' ? m.id : m);
            }
        }
        
        if (models.length > 0) {
            // 显示模型选择列表
            renderApiModelSelectionList(models, {
                prefix: cleanPrefix,
                api_key: apiKey,
                api_type: apiType,
                base_url: baseUrl,
                model_mappings: modelMappings,
                endpoint_id: selectedEndpointId
            });
            if (modelsModalFooter) {
                modelsModalFooter.style.display = 'block';
            }
        } else {
            if (modelsList) {
                modelsList.innerHTML = `<p style="color: var(--danger); padding: 16px; text-align: center;">获取模型列表失败</p>`;
            }
        }
    } catch (e) {
        if (modelsList) {
            modelsList.innerHTML = `<p style="color: var(--danger); padding: 16px; text-align: center;">请求失败: ${escapeHtml(e.message)}</p>`;
        }
    }
}

// 渲染对外接口模型选择列表
function renderApiModelSelectionList(models, apiData) {
    const container = document.getElementById('models-list');
    if (!container) return;
    
    container.innerHTML = models.map((m, index) => {
        // 处理字符串数组（映射模式）和对象数组（透传模式）
        const modelId = typeof m === 'object' ? m.id : m;
        const ownedBy = typeof m === 'object' ? m.owned_by : null;
        
        return `
            <div style="display: flex; align-items: center; padding: 10px 12px; background: var(--bg-tertiary); border-radius: var(--radius-sm); margin-bottom: 6px; cursor: pointer;" onclick="this.querySelector('input').checked = true;">
                <input type="radio" name="selected-model" value="${escapeAttr(modelId)}" ${index === 0 ? 'checked' : ''} style="margin-right: 12px;">
                <span style="flex: 1; font-family: var(--font-mono); font-size: 0.8125rem;">${escapeHtml(modelId)}</span>
                ${ownedBy ? `<span style="font-size: 0.75rem; color: var(--text-tertiary);">${escapeHtml(ownedBy)}</span>` : ''}
            </div>
        `;
    }).join('');
    
    container.dataset.apiData = JSON.stringify(apiData);
}

// 确认对外接口模型选择并进行对话测试
async function confirmApiModelAndTest() {
    const selectedModel = document.querySelector('input[name="selected-model"]:checked');
    if (!selectedModel) {
        showToast('请选择一个模型', 'error');
        return;
    }
    
    const container = document.getElementById('models-list');
    const apiData = JSON.parse(container.dataset.apiData || '{}');
    
    hideModal('models-modal');
    
    const testResult = document.getElementById('api-test-result');
    
    if (testResult) {
        testResult.style.display = 'block';
        testResult.style.background = 'rgba(33, 150, 243, 0.1)';
        testResult.style.border = '1px solid rgba(33, 150, 243, 0.3)';
        testResult.innerHTML = `
            <div style="color: #2196f3; font-weight: 500;">⟳ 正在测试模型: ${escapeHtml(selectedModel.value)}</div>
        `;
    }
    
    try {
        // 使用关联池中的端点进行测试
        const poolId = document.getElementById('api-pool').value;
        const statsRes = await fetch(`${API_BASE}/stats`);
        const stats = await statsRes.json();
        const poolEndpoints = (stats.endpoints || []).filter(ep => (ep.pool_ids || []).includes(poolId));
        
        if (poolEndpoints.length === 0) {
            if (testResult) {
                testResult.style.background = 'rgba(244, 67, 54, 0.1)';
                testResult.style.border = '1px solid rgba(244, 67, 54, 0.3)';
                testResult.innerHTML = `
                    <div style="color: #f44336; font-weight: 500;">✗ 测试失败</div>
                    <div style="font-size: 0.8125rem; color: var(--text-secondary); margin-top: 4px;">关联池中没有端点</div>
                `;
            }
            return;
        }
        
        // 使用存储的端点ID获取完整信息
        const endpointId = apiData.endpoint_id;
        if (!endpointId) {
            throw new Error('未选择端点');
        }
        const epRes = await fetch(`${API_BASE}/endpoints/${endpointId}`);
        if (!epRes.ok) {
            throw new Error('获取端点信息失败');
        }
        const fullEp = await epRes.json();
        
        // 根据映射关系转换模型名称
        let testModel = selectedModel.value;
        const modelMappings = apiData.model_mappings || [];
        const mapping = modelMappings.find(m => m.client_model === selectedModel.value);
        if (mapping) {
            testModel = mapping.endpoint_model;
        }
        
        // 使用后端的 check 接口进行测试
        const checkRes = await fetch(`${API_BASE}/endpoints/check`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
                name: fullEp.config.name,
                url: fullEp.config.url,
                api_type: fullEp.config.api_type,
                api_key: fullEp.config.api_key,
                token_limit: 1000,
                reset_policy: 'manual',
                enabled: true,
                model: testModel
            })
        });
        const result = await checkRes.json();
        
        if (testResult) {
            if (result.success) {
                testResult.style.background = 'rgba(76, 175, 80, 0.1)';
                testResult.style.border = '1px solid rgba(76, 175, 80, 0.3)';
                const modelInfo = testModel !== selectedModel.value 
                    ? `模型: ${escapeHtml(selectedModel.value)} → ${escapeHtml(testModel)}`
                    : `模型: ${escapeHtml(selectedModel.value)}`;
                testResult.innerHTML = `
                    <div style="color: #4caf50; font-weight: 500;">✓ 对话测试成功</div>
                    <div style="font-size: 0.75rem; color: var(--text-tertiary); margin-top: 4px;">${modelInfo}</div>
                    <div style="margin-top: 8px; padding: 12px; background: var(--bg-secondary); border-radius: var(--radius-sm);">
                        <div style="font-size: 0.75rem; color: var(--text-tertiary); margin-bottom: 4px;">模型回复:</div>
                        <div style="font-size: 0.875rem; color: var(--text-primary); line-height: 1.5;">${escapeHtml(result.message)}</div>
                    </div>
                `;
            } else {
                testResult.style.background = 'rgba(244, 67, 54, 0.1)';
                testResult.style.border = '1px solid rgba(244, 67, 54, 0.3)';
                const modelInfo = testModel !== selectedModel.value 
                    ? `模型: ${escapeHtml(selectedModel.value)} → ${escapeHtml(testModel)}`
                    : `模型: ${escapeHtml(selectedModel.value)}`;
                testResult.innerHTML = `
                    <div style="color: #f44336; font-weight: 500;">✗ 对话测试失败</div>
                    <div style="font-size: 0.75rem; color: var(--text-tertiary); margin-top: 4px;">${modelInfo}</div>
                    <div style="font-size: 0.8125rem; color: var(--text-secondary); margin-top: 4px;">
                        ${result.message}
                        ${result.tested_url ? `<br>测试 URL: <code style="font-size: 0.75rem; background: var(--bg-secondary); padding: 2px 4px; border-radius: 3px;">${escapeHtml(result.tested_url)}</code>` : ''}
                    </div>
                `;
            }
        }
        
        showToast(result.success ? '对话测试成功' : result.message, result.success ? 'success' : 'error');
    } catch (e) {
        if (testResult) {
            testResult.style.background = 'rgba(244, 67, 54, 0.1)';
            testResult.style.border = '1px solid rgba(244, 67, 54, 0.3)';
            testResult.innerHTML = `
                <div style="color: #f44336; font-weight: 500;">✗ 请求失败</div>
                <div style="font-size: 0.8125rem; color: var(--text-secondary); margin-top: 4px;">${escapeHtml(e.message)}</div>
            `;
        }
        showToast('请求失败: ' + e.message, 'error');
    }
}

// 更新对外接口完整 URL 显示
function updateApiFullUrlDisplay() {
    const prefix = document.getElementById('api-prefix').value.trim();
    const type = document.getElementById('api-type').value;
    const fullUrlDiv = document.getElementById('api-full-url');
    if (!fullUrlDiv) return;
    
    if (!prefix) {
        fullUrlDiv.textContent = '';
        return;
    }
    
    const baseUrl = window.location.origin;
    const cleanPrefix = prefix.startsWith('/') ? prefix : '/' + prefix;
    
    let examplePath = '';
    switch (type) {
        case 'openai':
            examplePath = '/chat/completions';
            break;
        case 'anthropic':
            examplePath = '/messages';
            break;
        case 'openai-responses':
            examplePath = '/responses';
            break;
        default:
            examplePath = '/chat/completions';
    }
    
    fullUrlDiv.textContent = `完整调用: ${baseUrl}${cleanPrefix}${examplePath}`;
}

// 保存对外API
async function handleSaveApi(e) {
    e.preventDefault();
    const id = document.getElementById('api-id').value;
    const data = {
        name: document.getElementById('api-name').value,
        prefix: document.getElementById('api-prefix').value,
        api_type: document.getElementById('api-type').value,
        pool_id: document.getElementById('api-pool').value,
        api_key: document.getElementById('api-key').value || null,
        enabled: document.getElementById('api-enabled').checked
    };

    try {
        const url = id ? `${API_BASE}/exposed-apis/${id}` : `${API_BASE}/exposed-apis`;
        const method = id ? 'PUT' : 'POST';
        
        const res = await fetch(url, {
            method,
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(data)
        });

        if (res.ok) {
            showToast(id ? '接口已更新' : '接口已添加', 'success');
            hideModal('api-modal');
            loadApisPage();
        } else {
            const err = await res.json();
            showToast(err.error?.message || '操作失败', 'error');
        }
    } catch (e) {
        showToast('网络错误', 'error');
    }
}

// 切换对外API状态
async function toggleApi(id) {
    try {
        const res = await fetch(`${API_BASE}/exposed-apis/${id}/toggle`, { method: 'POST' });
        if (res.ok) {
            showToast('接口状态已切换', 'success');
            loadApisPage();
        }
    } catch (e) {
        showToast('操作失败', 'error');
    }
}

// 删除对外API
async function deleteApi(id) {
    if (!confirm('确定要删除此对外接口吗？')) return;
    try {
        const res = await fetch(`${API_BASE}/exposed-apis/${id}`, { method: 'DELETE' });
        if (res.ok) {
            showToast('接口已删除', 'success');
            loadApisPage();
        }
    } catch (e) {
        showToast('操作失败', 'error');
    }
}

// 编辑池
async function editPool(id) {
    const pool = currentPools.find(p => p.id === id);
    if (!pool) return;
    
    document.getElementById('pool-modal-title').textContent = '编辑端点池';
    document.getElementById('pool-id').value = pool.id;
    document.getElementById('pool-name').value = pool.name;
    document.getElementById('pool-desc').value = pool.description || '';
    document.getElementById('pool-algorithm').value = pool.schedule_algorithm;
    document.getElementById('pool-model-mode').value = pool.model_mode || 'passthrough';
    document.getElementById('pool-retry-mode').value = pool.retry_mode || 'none';
    document.getElementById('pool-retry-count').value = pool.retry_count || 1;
    
    // 更新算法说明
    updatePoolAlgoDescription();
    
    // 更新模型模式说明
    updateModelModeDescription();
    
    // 更新重试模式说明和次数显示
    updateRetryModeDescription();
    
    // 更新端点映射配置显示
    updatePoolEndpointsMapping(id, pool.model_mode);
    
    // 监听模型模式变化
    const modelModeSelect = document.getElementById('pool-model-mode');
    modelModeSelect.onchange = () => {
        updatePoolEndpointsMapping(id, modelModeSelect.value);
        updateModelModeDescription();
    };
    
    // 监听重试模式变化
    const retryModeSelect = document.getElementById('pool-retry-mode');
    retryModeSelect.onchange = updateRetryModeDescription;
    
    showModal('pool-modal');
}

// 更新池端点映射配置显示
async function updatePoolEndpointsMapping(poolId, modelMode) {
    const container = document.getElementById('pool-endpoints-mapping');
    if (!container) return;
    
    // 只在映射模式下显示
    if (modelMode !== 'mapping') {
        container.style.display = 'none';
        return;
    }
    
    container.style.display = 'block';
    
    // 获取池中的端点
    const poolEndpoints = currentEndpoints.filter(ep => (ep.pool_ids || []).includes(poolId));
    
    if (poolEndpoints.length === 0) {
        container.innerHTML = '<p style="color: var(--text-tertiary); font-size: 0.875rem;">池中暂无端点</p>';
        return;
    }
    
    // 渲染端点列表
    let html = '<div style="font-size: 0.875rem; color: var(--text-secondary); margin-bottom: 12px;">端点模型映射配置</div>';
    
    for (const ep of poolEndpoints) {
        // 获取端点完整信息（包含映射）
        try {
            const res = await fetch(`${API_BASE}/endpoints/${ep.id}`);
            if (res.ok) {
                const fullEp = await res.json();
                const mappings = fullEp.config.model_mappings || [];
                const mappingText = mappings.length > 0 
                    ? mappings.map(m => `${m.client_model} → ${m.endpoint_model}`).join(', ')
                    : '未配置';
                
                html += `
                    <div style="display: flex; justify-content: space-between; align-items: center; padding: 8px; background: var(--bg-tertiary); border-radius: var(--radius-sm); margin-bottom: 8px;">
                        <div>
                            <span style="font-weight: 500;">${escapeHtml(ep.name)}</span>
                            <span style="font-size: 0.75rem; color: var(--text-tertiary); margin-left: 8px;">映射: ${escapeHtml(mappingText)}</span>
                        </div>
                        <button type="button" class="btn btn-small" onclick="editEndpointMappingFromPool('${escapeAttr(ep.id)}')">编辑映射</button>
                    </div>
                `;
            }
        } catch (e) {
            console.error('获取端点信息失败:', e);
        }
    }
    
    container.innerHTML = html;
}

// 从池编辑页面打开端点映射编辑
async function editEndpointMappingFromPool(endpointId) {
    // 先关闭池编辑对话框
    hideModal('pool-modal');
    
    // 打开端点映射对话框
    await showEndpointMappingModal(endpointId);
}

// 保存池
async function handleSavePool(e) {
    e.preventDefault();
    const id = document.getElementById('pool-id').value;
    const data = {
        name: document.getElementById('pool-name').value,
        description: document.getElementById('pool-desc').value || null,
        schedule_algorithm: document.getElementById('pool-algorithm').value,
        model_mode: document.getElementById('pool-model-mode').value,
        retry_mode: document.getElementById('pool-retry-mode').value,
        retry_count: parseInt(document.getElementById('pool-retry-count').value) || 1
    };

    try {
        const url = id ? `${API_BASE}/pools/${id}` : `${API_BASE}/pools`;
        const method = id ? 'PUT' : 'POST';
        
        const res = await fetch(url, {
            method,
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(data)
        });

        if (res.ok) {
            showToast(id ? '池已更新' : '池已添加', 'success');
            hideModal('pool-modal');
            loadApisPage();
        } else {
            const err = await res.json();
            showToast(err.error?.message || '操作失败', 'error');
        }
    } catch (e) {
        showToast('网络错误', 'error');
    }
}

// 删除池
async function deletePool(id) {
    if (!confirm('确定要删除此池吗？关联的端点和对外接口将被解除关联。')) return;
    try {
        const res = await fetch(`${API_BASE}/pools/${id}`, { method: 'DELETE' });
        if (res.ok) {
            showToast('池已删除', 'success');
            loadApisPage();
        }
    } catch (e) {
        showToast('操作失败', 'error');
    }
}

// 更新池模态框中的算法说明
function updatePoolAlgoDescription() {
    const select = document.getElementById('pool-algorithm');
    if (!select) return;
    
    const selectedAlgo = select.value;
    const container = document.getElementById('pool-algo-desc');
    if (!container) return;
    
    const items = container.querySelectorAll('.algo-item');
    items.forEach(item => {
        item.style.display = item.dataset.algo === selectedAlgo ? 'block' : 'none';
    });
}

// 更新模型模式说明
function updateModelModeDescription() {
    const select = document.getElementById('pool-model-mode');
    if (!select) return;
    
    const selectedMode = select.value;
    const container = document.getElementById('model-mode-desc');
    if (!container) return;
    
    const items = container.querySelectorAll('.model-mode-item');
    items.forEach(item => {
        item.style.display = item.dataset.mode === selectedMode ? 'block' : 'none';
    });
}

// 更新重试模式说明
function updateRetryModeDescription() {
    const select = document.getElementById('pool-retry-mode');
    if (!select) return;
    
    const selectedMode = select.value;
    const container = document.getElementById('retry-mode-desc');
    if (!container) return;
    
    const items = container.querySelectorAll('.retry-mode-item');
    items.forEach(item => {
        item.style.display = item.dataset.mode === selectedMode ? 'block' : 'none';
    });
    
    // 更新重试次数输入框显示
    const countGroup = document.getElementById('retry-count-group');
    if (countGroup) {
        countGroup.style.display = selectedMode === 'none' ? 'none' : 'block';
    }
}

// 消息提示
function showToast(message, type = 'success') {
    const toast = document.getElementById('toast');
    toast.textContent = message;
    toast.className = `toast ${type}`;
    toast.style.display = 'block';
    toast.classList.remove('hiding');
    
    // 清除之前的定时器
    if (toast._hideTimer) clearTimeout(toast._hideTimer);
    
    // 3秒后自动隐藏（带动画）
    toast._hideTimer = setTimeout(() => {
        toast.classList.add('hiding');
        setTimeout(() => {
            toast.style.display = 'none';
            toast.classList.remove('hiding');
        }, 300);
    }, 3000);
}

// 错误提示
function showError(id, message) {
    const el = document.getElementById(id);
    el.textContent = message;
    el.style.display = 'block';
    setTimeout(() => {
        el.style.display = 'none';
    }, 5000);
}

// 工具函数
function formatNumber(num) {
    if (num >= 1000000) return (num / 1000000).toFixed(1) + 'M';
    if (num >= 1000) return (num / 1000).toFixed(1) + 'K';
    return num.toString();
}

// 格式化限额数字（接近12个9时直接显示）
function formatLimit(num) {
    // 大于 999999999000 时显示为无上限
    if (num >= 999999999000) return '无上限';
    return formatNumber(num);
}

function truncate(str, len) {
    return str.length > len ? str.substring(0, len) + '...' : str;
}

function escapeHtml(str) {
    if (!str) return '';
    const div = document.createElement('div');
    div.textContent = String(str);
    return div.innerHTML;
}

// 转义用于 onclick 属性的字符串（防止 XSS）
function escapeAttr(str) {
    if (!str) return '';
    return String(str).replace(/\\/g, '\\\\').replace(/'/g, "\\'").replace(/"/g, '&quot;');
}

// 格式化日期时间（本地时间）
function formatDateTime(isoString) {
    if (!isoString) return '-';
    const date = new Date(isoString);
    if (isNaN(date.getTime())) return isoString;
    const pad = n => String(n).padStart(2, '0');
    return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
}

// ========== 调用日志页面 ==========

async function loadCallLogs() {
    const tbody = document.getElementById('call-logs-body');
    if (!tbody) return;

    tbody.innerHTML = '<tr><td colspan="10" style="text-align: center; color: var(--text-secondary);">加载中...</td></tr>';

    try {
        const res = await fetch(`${API_BASE}/logs`);
        if (!res.ok) {
            throw new Error(`HTTP ${res.status}`);
        }
        const data = await res.json();
        renderCallLogs(data.logs || []);
    } catch (e) {
        console.error('加载调用日志失败:', e);
        tbody.innerHTML = `<tr><td colspan="10" style="text-align: center; color: var(--danger);">加载失败: ${escapeHtml(e.message)}</td></tr>`;
    }
}

function renderCallLogs(logs) {
    const tbody = document.getElementById('call-logs-body');
    if (!tbody) return;

    if (!logs || logs.length === 0) {
        tbody.innerHTML = '<tr><td colspan="10" style="text-align: center; color: var(--text-secondary);">暂无调用日志</td></tr>';
        return;
    }

    tbody.innerHTML = logs.map(log => {
        const statusClass = log.status === 'success' ? 'status-success' : 'status-error';
        const statusText = log.status === 'success' ? '成功' : '失败';
        const errorMsg = log.error_message ? `<span class="log-error" title="${escapeAttr(log.error_message)}">${escapeHtml(log.error_message)}</span>` : '-';
        return `
            <tr>
                <td>${escapeHtml(formatDateTime(log.timestamp))}</td>
                <td>${escapeHtml(log.client_ip || '-')}</td>
                <td>${escapeHtml(log.method || '-')}</td>
                <td>${escapeHtml(log.path || '-')}</td>
                <td>${escapeHtml(log.api_prefix || '-')}</td>
                <td>${escapeHtml(log.endpoint_name || log.endpoint_id || '-')}</td>
                <td>${log.status_code || '-'}</td>
                <td><span class="${statusClass}">${statusText}</span></td>
                <td>${log.duration_ms !== undefined ? log.duration_ms : '-'}</td>
                <td>${errorMsg}</td>
            </tr>
        `;
    }).join('');
}

// ========== 延迟排行榜 ==========

async function loadLatencyLeaderboard() {
    const tbody = document.getElementById('latency-leaderboard-body');
    if (!tbody) return;

    tbody.innerHTML = '<tr><td colspan="10" style="text-align: center; color: var(--text-secondary);">加载中...</td></tr>';

    try {
        const res = await fetch(`${API_BASE}/latency-leaderboard`);
        if (!res.ok) {
            throw new Error(`HTTP ${res.status}`);
        }
        const data = await res.json();
        renderLatencyLeaderboard(data.leaderboard || []);
    } catch (e) {
        console.error('加载延迟排行榜失败:', e);
        tbody.innerHTML = `<tr><td colspan="10" style="text-align: center; color: var(--danger);">加载失败: ${escapeHtml(e.message)}</td></tr>`;
    }
}

function renderLatencyLeaderboard(stats) {
    const tbody = document.getElementById('latency-leaderboard-body');
    if (!tbody) return;

    if (!stats || stats.length === 0) {
        tbody.innerHTML = '<tr><td colspan="10" style="text-align: center; color: var(--text-secondary);">暂无延迟数据</td></tr>';
        return;
    }

    tbody.innerHTML = stats.map((item, index) => {
        const rank = index + 1;
        const rankClass = rank === 1 ? 'rank-1' : rank === 2 ? 'rank-2' : rank === 3 ? 'rank-3' : '';
        const enabledText = item.enabled ? '启用' : '禁用';
        const enabledClass = item.enabled ? 'status-active' : 'status-disabled';
        const formatMs = (ms) => ms > 0 ? `${ms}ms` : '-';
        return `
            <tr>
                <td><span class="latency-rank ${rankClass}">${rank}</span></td>
                <td>${escapeHtml(item.endpoint_name)}</td>
                <td><span class="${enabledClass}">${enabledText}</span></td>
                <td>${item.samples || 0}</td>
                <td><strong>${formatMs(item.avg_ms)}</strong></td>
                <td>${formatMs(item.min_ms)}</td>
                <td>${formatMs(item.max_ms)}</td>
                <td>${formatMs(item.p50_ms)}</td>
                <td>${formatMs(item.p90_ms)}</td>
                <td>${formatMs(item.p95_ms)}</td>
            </tr>
        `;
    }).join('');
}
