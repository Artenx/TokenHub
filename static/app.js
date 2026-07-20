// API 基础路径
const API_BASE = '/admin/api';

// 状态
let currentEndpoints = [];
let currentConfig = {};
let currentPools = [];
let currentApis = [];
let endpointSearchTerm = '';
let currentReplayApiId = null;
let currentReplayConfig = { max_records_per_api: 50, state_file_path: 'replay_state.json', max_body_size_kb: 1024 };
let benchmarkTargets = [];
let currentSkillSources = [];
const builtinBenchmarkCases = [
    { id: 'math-discount', category: '数学推理', name: '折扣与税费计算', content: '一件商品标价 240 元，先打八折，再按折后价格加收 6% 税费。请列出计算步骤并给出最终价格。' },
    { id: 'logic-seating', category: '逻辑推理', name: '座位排列推理', content: '甲、乙、丙、丁四人从左到右坐成一排。甲不坐两端，乙坐在丙左侧，丁不与甲相邻。请给出一种满足条件的排列，并简要说明。' },
    { id: 'json-instruction', category: '约束遵循', name: '严格 JSON 输出', content: '从文本“订单 A-102，数量 3，单价 19.8，客户为李明”提取订单号、数量、单价和客户。只输出一个 JSON 对象，键名使用 order_id、quantity、unit_price、customer。' },
    { id: 'code-debug', category: '代码', name: 'JavaScript 调试', content: '以下 JavaScript 函数期望返回数组中最大的偶数：\n```js\nfunction maxEven(values) {\n  return values.filter(v => v % 2).sort()[0];\n}\n```\n请指出两个问题，并给出修复后的函数。' },
    { id: 'sql-query', category: 'SQL', name: '聚合查询', content: '表 orders 包含 customer_id、amount、created_at 字段。请写一条 PostgreSQL 查询，返回 2025 年每位客户的订单数和总消费额，并按总消费额降序排列。' },
    { id: 'algorithm', category: '算法', name: '区间合并', content: '给定按起点排序的闭区间数组，例如 [[1,3],[2,6],[8,10],[15,18]]，请说明合并重叠区间的算法、时间复杂度，并给出 Python 实现。' },
    { id: 'extraction', category: '信息抽取', name: '会议要点提取', content: '从以下文本提取行动项、负责人和截止日期，使用 Markdown 表格：\n“周会决定：王晨在周五前完成登录页；陈雨在下周二前核对支付接口；李娜负责整理测试清单，截止本月 30 日。”' },
    { id: 'summary', category: '摘要', name: '技术摘要', content: '将以下内容压缩为三条要点，每条不超过 25 个字：\n“系统将请求按端点与模型组合执行。每次请求记录首字节延迟、总耗时和 Token。完成后由评审模型给出多维评分，结果长期保存供后续比较。”' },
    { id: 'translation', category: '翻译', name: '中英翻译', content: '将下面英文翻译成自然、专业的中文：\n“Reliable evaluation requires consistent inputs, transparent metrics, and repeatable execution across all candidates.”' },
    { id: 'creative-constraint', category: '创作', name: '约束写作', content: '写一段 80 至 100 字的产品更新通知，主题是“模型评测功能上线”。内容需包含性能比较、自动评分和自定义样本三个要点，语气专业直接。' },
];

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
    const urlHint = document.getElementById('ep-url-hint');
    
    if (!epUrl || !epType || !fullUrlDiv) return;
    
    const baseUrl = epUrl.value.trim();
    const apiType = epType.value;
    
    // 更新提示文本
    if (urlHint) {
        urlHint.textContent = apiType === 'custom'
            ? '输入完整的 API 请求 URL'
            : '只需输入基础路径，我会自动补全完整路径';
    }
    
    if (!baseUrl) {
        fullUrlDiv.textContent = '';
        return;
    }
    
    // 自定义类型：直接显示输入的完整 URL，不做任何补全
    if (apiType === 'custom') {
        fullUrlDiv.textContent = '完整路径: ' + baseUrl;
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
    let urlPath;
    try {
        urlPath = new URL(cleanBase).pathname;
    } catch {
        fullUrlDiv.textContent = '完整路径: ' + cleanBase + '/v1' + endpoint;
        return;
    }
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
    document.getElementById('btn-refresh-benchmarks')?.addEventListener('click', loadModelBenchmarks);
    document.getElementById('benchmark-form')?.addEventListener('submit', createModelBenchmark);
    document.getElementById('benchmark-endpoints')?.addEventListener('change', updateBenchmarkCandidateModels);
    document.getElementById('benchmark-judge-endpoint')?.addEventListener('change', updateBenchmarkJudgeModels);
    document.getElementById('btn-add-benchmark-target')?.addEventListener('click', addBenchmarkTarget);
    document.getElementById('btn-select-all-benchmark-cases')?.addEventListener('click', toggleAllBuiltinBenchmarkCases);
    document.getElementById('btn-import-benchmark-cases')?.addEventListener('click', importBuiltinBenchmarkCases);
    renderBuiltinBenchmarkCases();
    document.querySelectorAll('.benchmark-tab').forEach(btn => btn.addEventListener('click', () => switchBenchmarkView(btn.dataset.benchmarkView)));
    document.getElementById('btn-refresh-skills')?.addEventListener('click', loadLocalSkills);
    document.getElementById('skill-upload-input')?.addEventListener('change', previewSkillUpload);
    document.getElementById('skill-search-form')?.addEventListener('submit', searchSkills);
    document.querySelectorAll('.skill-tab').forEach(btn => btn.addEventListener('click', () => switchSkillView(btn.dataset.skillView)));
    document.getElementById('btn-add-skill-source')?.addEventListener('click', addCustomSkillSource);
    document.getElementById('btn-save-skill-sources')?.addEventListener('click', saveSkillSources);

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

    // 池测试端点选择器切换时重新加载模型列表
    const poolTestEndpointSelect = document.getElementById('pool-test-endpoint-select');
    if (poolTestEndpointSelect) {
        poolTestEndpointSelect.addEventListener('change', () => {
            loadPoolTestModelsForEndpoint(poolTestEndpointSelect.value);
        });
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

    // 测试端点选择器切换时重新加载模型列表
    const testEndpointSelect = document.getElementById('test-endpoint-select');
    if (testEndpointSelect) {
        testEndpointSelect.addEventListener('change', () => {
            const container = document.getElementById('models-list');
            if (container && container.dataset.apiTestContext) {
                loadApiTestModelsForSelected();
            } else {
                clearApiTestData();
                loadModelsForSelectedEndpoint();
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

    const replayConfigForm = document.getElementById('replay-config-form');
    if (replayConfigForm) replayConfigForm.addEventListener('submit', saveReplayConfig);
    document.getElementById('btn-refresh-replay')?.addEventListener('click', loadReplayRecords);
    document.getElementById('btn-clear-replay')?.addEventListener('click', clearReplayRecords);

    // 添加端点按钮（端点列表页面）
    const btnAddEndpoint = document.getElementById('btn-add-endpoint');
    if (btnAddEndpoint) {
        btnAddEndpoint.addEventListener('click', () => {
            addEndpointToPool('');
        });
    }

    // 端点列表搜索框
    const endpointListSearch = document.getElementById('endpoint-list-search');
    if (endpointListSearch) {
        endpointListSearch.addEventListener('input', (e) => {
            endpointSearchTerm = e.target.value;
            renderEndpointsList();
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
        document.getElementById('api-name-warning').style.display = 'none';
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
        document.getElementById('pool-name-warning').style.display = 'none';
        showModal('pool-modal');
    });

    // 池表单
    document.getElementById('pool-form').addEventListener('submit', handleSavePool);

    // 名称重复即时校验
    const epNameInput = document.getElementById('ep-name');
    if (epNameInput) {
        epNameInput.addEventListener('input', () => {
            const id = document.getElementById('ep-id').value;
            checkDuplicateName(epNameInput.value, currentEndpoints, id, 'ep-name-warning');
        });
    }
    const poolNameInput = document.getElementById('pool-name');
    if (poolNameInput) {
        poolNameInput.addEventListener('input', () => {
            const id = document.getElementById('pool-id').value;
            checkDuplicateName(poolNameInput.value, currentPools, id, 'pool-name-warning');
        });
    }
    const apiNameInput = document.getElementById('api-name');
    if (apiNameInput) {
        apiNameInput.addEventListener('input', () => {
            const id = document.getElementById('api-id').value;
            checkDuplicateName(apiNameInput.value, currentApis, id, 'api-name-warning');
        });
    }

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
        document.getElementById('stat-total-consumed').textContent = formatNumber(stats.total_tokens_consumed);
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
                        <label>已用Token/Token限额</label>
                        <span>${formatNumber(ep.tokens_used)} / ${formatLimit(ep.token_limit)}</span>
                    </div>
                    <div class="endpoint-detail">
                        <label>已请求/请求限额</label>
                        <span>${formatNumber(ep.requests_used)} / ${ep.request_limit > 0 ? formatNumber(ep.request_limit) : '无上限'}</span>
                    </div>
                    <div class="endpoint-detail">
                        <label>总消耗Token</label>
                        <span>${formatNumber(ep.total_tokens_consumed)}</span>
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

    const searchTerm = endpointSearchTerm.trim().toLowerCase();
    const filteredEndpoints = searchTerm
        ? currentEndpoints.filter(ep =>
            ep.name.toLowerCase().includes(searchTerm) ||
            (ep.url && ep.url.toLowerCase().includes(searchTerm))
          )
        : currentEndpoints;

    if (filteredEndpoints.length === 0 && currentEndpoints.length === 0) {
        container.innerHTML = '<p style="color: var(--text-secondary);">暂无端点，点击"添加端点"开始</p>';
        return;
    }

    if (filteredEndpoints.length === 0) {
        container.innerHTML = '<p style="color: var(--text-secondary);">未找到匹配的端点</p>';
        return;
    }

    container.innerHTML = [...filteredEndpoints]
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
                        ${api.replay_enabled ? '<span class="status-badge replay-badge">回放中</span>' : ''}
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

// 端点卡片对话测试 - 先获取模型列表让用户选择
async function quickTestEndpoint(id, name) {
    const modelsList = document.getElementById('models-list');
    const modelsModalTitle = document.getElementById('models-modal-title');
    const modelsModalFooter = document.getElementById('models-modal-footer');

    if (modelsList) {
        modelsList.innerHTML = '<p style="color: var(--text-secondary); padding: 16px; text-align: center;">加载模型列表...</p>';
    }
    if (modelsModalFooter) {
        modelsModalFooter.style.display = 'none';
    }
    if (modelsModalTitle) {
        modelsModalTitle.textContent = `选择测试模型 - ${escapeHtml(name)}`;
    }
    clearApiTestData();
    showTestEndpointSelector(false);
    showModal('models-modal');

    try {
        const epRes = await fetch(`${API_BASE}/endpoints/${encodeURIComponent(id)}`);
        if (!epRes.ok) throw new Error('获取端点信息失败');
        const fullEp = await epRes.json();
        const config = fullEp.config;

        const data = {
            name: config.name,
            url: config.url,
            api_type: config.api_type,
            api_key: config.api_key,
            token_limit: 1000,
            reset_policy: 'manual',
            enabled: true
        };

        const res = await fetch(`${API_BASE}/endpoints/models`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(data)
        });
        const result = await res.json();

        if (result.success && result.models && result.models.length > 0) {
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

// 添加端点到指定池
function addEndpointToPool(poolId) {
    document.getElementById('modal-title').textContent = '添加端点';
    document.getElementById('endpoint-form').reset();
    document.getElementById('ep-id').value = '';
    document.getElementById('ep-enabled').checked = true;
    document.getElementById('ep-name-warning').style.display = 'none';
    
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
    clearApiTestData();
    showTestEndpointSelector(false);
    showModal('models-modal');

    try {
        const res = await fetch(`${API_BASE}/endpoints/models`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(data)
        });
        const result = await res.json();
        
        if (result.success && result.models && result.models.length > 0) {
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

// ========== 测试模型选择（支持切换端点） ==========

// 填充测试端点下拉选择器
function populateTestEndpointSelector(selectedId) {
    const select = document.getElementById('test-endpoint-select');
    if (!select) return;
    
    const endpoints = currentEndpoints || [];
    select.innerHTML = endpoints.map(ep => 
        `<option value="${escapeAttr(ep.id)}" ${ep.id === selectedId ? 'selected' : ''}>${escapeHtml(ep.name)}</option>`
    ).join('');
}

// 用指定端点列表填充选择器（用于 API 测试，只显示池内端点）
function populateTestEndpointSelectorFromList(endpoints, selectedId) {
    const select = document.getElementById('test-endpoint-select');
    if (!select) return;
    
    const list = endpoints || [];
    select.innerHTML = list.map(ep => 
        `<option value="${escapeAttr(ep.id)}" ${ep.id === selectedId ? 'selected' : ''}>${escapeHtml(ep.name)}</option>`
    ).join('');
}

// 根据下拉选中的端点加载模型列表
async function loadModelsForSelectedEndpoint() {
    const select = document.getElementById('test-endpoint-select');
    const modelsList = document.getElementById('models-list');
    const modelsModalFooter = document.getElementById('models-modal-footer');
    
    const endpointId = select.value;
    if (!endpointId) {
        if (modelsList) {
            modelsList.innerHTML = '<p style="color: var(--text-secondary); padding: 16px; text-align: center;">请先选择一个端点</p>';
        }
        if (modelsModalFooter) {
            modelsModalFooter.style.display = 'none';
        }
        return;
    }
    
    if (modelsList) {
        modelsList.innerHTML = '<p style="color: var(--text-secondary); padding: 16px; text-align: center;">加载模型列表...</p>';
    }
    if (modelsModalFooter) {
        modelsModalFooter.style.display = 'none';
    }
    
    try {
        const epRes = await fetch(`${API_BASE}/endpoints/${encodeURIComponent(endpointId)}`);
        if (!epRes.ok) throw new Error('获取端点信息失败');
        const fullEp = await epRes.json();
        const config = fullEp.config;
        
        const data = {
            name: config.name,
            url: config.url,
            api_type: config.api_type,
            api_key: config.api_key,
            token_limit: 1000,
            reset_policy: 'manual',
            enabled: true
        };
        
        await loadModelsWithData(data);
    } catch (e) {
        if (modelsList) {
            modelsList.innerHTML = `<p style="color: var(--danger); padding: 16px; text-align: center;">请求失败: ${escapeHtml(e.message)}</p>`;
        }
    }
}

// 使用指定端点数据加载模型列表
async function loadModelsWithData(data) {
    const modelsList = document.getElementById('models-list');
    const modelsModalFooter = document.getElementById('models-modal-footer');
    
    try {
        const res = await fetch(`${API_BASE}/endpoints/models`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(data)
        });
        const result = await res.json();
        
        if (result.success && result.models && result.models.length > 0) {
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

// 清除 API 测试数据标记，确保确认按钮走端点测试路径
function clearApiTestData() {
    const container = document.getElementById('models-list');
    if (container) {
        delete container.dataset.apiData;
        delete container.dataset.apiTestContext;
    }
}

// 显示/隐藏测试端点选择器
function showTestEndpointSelector(show) {
    const select = document.getElementById('test-endpoint-select');
    if (select && select.parentElement) {
        select.parentElement.style.display = show ? 'block' : 'none';
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
        if (isActive) {
            const titleEl = document.getElementById('header-page-title');
            if (titleEl) titleEl.textContent = btn.textContent.trim();
        }
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
    } else if (tab === 'model-benchmarks') {
        loadModelBenchmarks();
    } else if (tab === 'skill-repository') {
        loadSkillRepository();
    } else if (tab === 'settings') {
        loadReplayConfig();
    }
}

function switchBenchmarkView(view) {
    document.getElementById('benchmark-tasks-view').style.display = view === 'tasks' ? '' : 'none';
    document.getElementById('benchmark-monitor-view').style.display = view === 'monitor' ? '' : 'none';
    document.querySelectorAll('.benchmark-tab').forEach(btn => btn.classList.toggle('active', btn.dataset.benchmarkView === view));
    if (view === 'monitor') loadLatencyLeaderboard();
}

async function createModelBenchmark(event) {
    event.preventDefault();
    let cases;
    try { cases = JSON.parse(document.getElementById('benchmark-samples').value); } catch { showToast('样本 JSON 格式错误', 'error'); return; }
    if (!Array.isArray(cases) || !cases.length) { showToast('至少提供一条样本', 'error'); return; }
    if (benchmarkTargets.length < 2) { showToast('至少添加两个被测模型组合', 'error'); return; }
    const body = { targets: benchmarkTargets, cases: cases.map((item, index) => ({ id: item.id || `case-${index + 1}`, name: item.name || `样本 ${index + 1}`, messages: item.messages, system_prompt: item.system_prompt || null })), judge: { endpoint_id: document.getElementById('benchmark-judge-endpoint').value, model: document.getElementById('benchmark-judge-model').value, rubric: document.getElementById('benchmark-rubric').value.trim() } };
    try {
        const response = await fetch(`${API_BASE}/model-benchmarks`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) });
        if (!response.ok) throw new Error(await response.text());
        showToast('模型评测任务已创建', 'success');
        await loadModelBenchmarks();
    } catch (error) { showToast(`创建评测失败: ${error.message}`, 'error'); }
}

async function loadModelBenchmarks() {
    const container = document.getElementById('benchmark-list');
    if (!container) return;
    try {
        const benchmarkResponse = await fetch(`${API_BASE}/model-benchmarks`);
        if (!benchmarkResponse.ok) throw new Error('加载失败');
        const runs = await benchmarkResponse.json();
        await populateBenchmarkEndpointOptions();
        container.innerHTML = runs.length ? runs.slice().reverse().map(run => {
            const targets = run.targets?.length ? run.targets : run.endpoint_ids.map(endpoint_id => ({ endpoint_id, model: run.model }));
            return `<article class="benchmark-run"><div class="benchmark-run-meta"><div class="benchmark-run-header"><span class="status-badge ${benchmarkStatusClass(run.status)}">${benchmarkStatusText(run.status)}</span><span>${targets.length} 个组合</span><span>${run.cases.length} 条样本</span><span>${new Date(run.created_at).toLocaleString()}</span></div><div class="benchmark-run-targets">${targets.map(target => `<span>${escapeHtml(target.model)}</span>`).join('')}</div></div><div class="benchmark-run-actions"><button class="btn btn-small" onclick="showModelBenchmark('${escapeAttr(run.id)}')">查看结果</button>${['queued','running'].includes(run.status) ? `<button class="btn btn-small btn-danger" onclick="cancelModelBenchmark('${escapeAttr(run.id)}')">取消</button>` : ''}</div></article>`;
        }).join('') : '<p class="benchmark-empty">暂无评测任务</p>';
    } catch { container.innerHTML = '<p style="color:var(--danger);">加载评测任务失败</p>'; }
}

async function populateBenchmarkEndpointOptions() {
    const candidateSelect = document.getElementById('benchmark-endpoints');
    const judgeSelect = document.getElementById('benchmark-judge-endpoint');
    if (!candidateSelect || !judgeSelect) return;
    const response = await fetch(`${API_BASE}/model-benchmarks/candidates`);
    if (!response.ok) throw new Error('加载端点模型失败');
    const candidates = (await response.json()).filter(endpoint => endpoint.enabled);
    const selectedCandidate = candidateSelect.value;
    const selectedJudge = judgeSelect.value;
    const options = candidates.map(endpoint => `<option value="${escapeAttr(endpoint.id)}" ${endpoint.id === selectedCandidate ? 'selected' : ''}>${escapeHtml(endpoint.name)}${endpoint.models.length ? ` · ${endpoint.models.length} 个模型` : ' · 未发现模型'}</option>`).join('');
    candidateSelect.innerHTML = options;
    judgeSelect.innerHTML = candidates.map(endpoint => `<option value="${escapeAttr(endpoint.id)}" ${endpoint.id === selectedJudge ? 'selected' : ''}>${escapeHtml(endpoint.name)}${endpoint.models.length ? ` · ${endpoint.models.length} 个模型` : ' · 未发现模型'}</option>`).join('');
    candidateSelect.dataset.models = JSON.stringify(candidates);
    judgeSelect.dataset.models = JSON.stringify(candidates);
    updateBenchmarkCandidateModels();
    updateBenchmarkJudgeModels();
    renderBenchmarkTargets();
}

function updateBenchmarkCandidateModels() {
    const endpointSelect = document.getElementById('benchmark-endpoints');
    const modelSelect = document.getElementById('benchmark-model');
    const candidates = JSON.parse(endpointSelect.dataset.models || '[]');
    const selectedModel = modelSelect.value;
    const models = (candidates.find(endpoint => endpoint.id === endpointSelect.value)?.models || []).slice().sort();
    modelSelect.disabled = !models.length;
    modelSelect.innerHTML = models.length ? `<option value="">选择被测模型</option>${models.map(model => `<option value="${escapeAttr(model)}" ${model === selectedModel ? 'selected' : ''}>${escapeHtml(model)}</option>`).join('')}` : '<option value="">该端点没有可用模型</option>';
}

function addBenchmarkTarget() {
    const endpoint = document.getElementById('benchmark-endpoints');
    const model = document.getElementById('benchmark-model');
    if (!endpoint.value || !model.value) { showToast('请选择被测端点和模型', 'error'); return; }
    if (benchmarkTargets.some(target => target.endpoint_id === endpoint.value && target.model === model.value)) { showToast('该端点与模型组合已添加', 'error'); return; }
    benchmarkTargets.push({ endpoint_id: endpoint.value, model: model.value });
    renderBenchmarkTargets();
}

function removeBenchmarkTarget(index) {
    benchmarkTargets.splice(index, 1);
    renderBenchmarkTargets();
}

function renderBenchmarkTargets() {
    const container = document.getElementById('benchmark-targets');
    const candidates = JSON.parse(document.getElementById('benchmark-endpoints').dataset.models || '[]');
    container.innerHTML = benchmarkTargets.map((target, index) => {
        const name = candidates.find(endpoint => endpoint.id === target.endpoint_id)?.name || target.endpoint_id;
        return `<span class="benchmark-target">${escapeHtml(name)} · ${escapeHtml(target.model)}<button type="button" onclick="removeBenchmarkTarget(${index})" aria-label="移除">×</button></span>`;
    }).join('') || '<small>尚未添加被测模型组合</small>';
}

function renderBuiltinBenchmarkCases() {
    const container = document.getElementById('benchmark-sample-library');
    if (!container) return;
    container.innerHTML = builtinBenchmarkCases.map(item => `<label class="benchmark-sample-option"><input type="checkbox" value="${escapeAttr(item.id)}"><span><strong>${escapeHtml(item.name)}</strong><small>${escapeHtml(item.category)}</small></span></label>`).join('');
}

function toggleAllBuiltinBenchmarkCases() {
    const inputs = Array.from(document.querySelectorAll('#benchmark-sample-library input[type="checkbox"]'));
    const selectAll = inputs.some(input => !input.checked);
    inputs.forEach(input => { input.checked = selectAll; });
    document.getElementById('btn-select-all-benchmark-cases').textContent = selectAll ? '取消全选' : '全选';
}

function importBuiltinBenchmarkCases() {
    const selectedIds = new Set(Array.from(document.querySelectorAll('#benchmark-sample-library input:checked')).map(input => input.value));
    const selected = builtinBenchmarkCases.filter(item => selectedIds.has(item.id));
    if (!selected.length) { showToast('请选择至少一条内置题目', 'error'); return; }
    const textarea = document.getElementById('benchmark-samples');
    let customCases = [];
    if (textarea.value.trim()) {
        try { customCases = JSON.parse(textarea.value); } catch { showToast('请先修正样本 JSON，再导入题目', 'error'); return; }
        if (!Array.isArray(customCases)) { showToast('样本 JSON 应为数组', 'error'); return; }
    }
    const existingIds = new Set(customCases.map(item => item.id));
    const additions = selected.filter(item => !existingIds.has(item.id) && !existingIds.has(`builtin-${item.id}`)).map(item => ({ id: `builtin-${item.id}`, name: item.name, messages: [{ role: 'user', content: item.content }] }));
    if (!additions.length) { showToast('所选题目已全部存在于样本中', 'error'); return; }
    textarea.value = JSON.stringify([...customCases, ...additions], null, 2);
    showToast(`已导入 ${additions.length} 条内置题目`, 'success');
}

function updateBenchmarkJudgeModels() {
    const endpointSelect = document.getElementById('benchmark-judge-endpoint');
    const modelSelect = document.getElementById('benchmark-judge-model');
    const candidates = JSON.parse(endpointSelect.dataset.models || '[]');
    const models = (candidates.find(endpoint => endpoint.id === endpointSelect.value)?.models || []).slice().sort();
    const selectedModel = modelSelect.value;
    modelSelect.disabled = !models.length;
    modelSelect.innerHTML = models.length ? `<option value="">选择评审模型</option>${models.map(model => `<option value="${escapeAttr(model)}" ${model === selectedModel ? 'selected' : ''}>${escapeHtml(model)}</option>`).join('')}` : '<option value="">该端点没有可用模型</option>';
}

function benchmarkStatusClass(status) {
    return { completed: 'active', failed: 'disabled', cancelled: 'disabled', running: 'exhausted', queued: 'exhausted' }[status] || 'exhausted';
}

function benchmarkStatusText(status) {
    return { completed: '已完成', failed: '失败', cancelled: '已取消', running: '执行中', queued: '排队中' }[status] || status;
}

function benchmarkJudgeStatus(judge) {
    if (!judge) return { text: '无评审', className: 'missing' };
    if (judge.status === 'success') return { text: '评审成功', className: 'success' };
    if (judge.status === 'judge_parse_error') return { text: '评审解析失败', className: 'error' };
    return { text: '评审失败', className: 'error' };
}

async function showModelBenchmark(id) {
    const detail = document.getElementById('benchmark-detail');
    try {
        const response = await fetch(`${API_BASE}/model-benchmarks/${id}`);
        if (!response.ok) throw new Error('加载失败');
        const data = await response.json();
        const summaries = data.summaries.slice().sort((left, right) => (right.average_score ?? -1) - (left.average_score ?? -1) || right.success_rate - left.success_rate || (left.median_duration_ms ?? Infinity) - (right.median_duration_ms ?? Infinity));
        const bestScore = summaries.find(summary => summary.average_score != null);
        const fastest = summaries.filter(summary => summary.median_duration_ms != null).sort((left, right) => left.median_duration_ms - right.median_duration_ms)[0];
        const mostReliable = summaries.slice().sort((left, right) => right.success_rate - left.success_rate)[0];
        const highlights = `<div class="benchmark-highlights">${bestScore ? `<div><span>最佳评分</span><strong>${escapeHtml(bestScore.model)} · ${bestScore.average_score.toFixed(1)}</strong></div>` : ''}${fastest ? `<div><span>最快响应</span><strong>${escapeHtml(fastest.model)} · ${fastest.median_duration_ms}ms</strong></div>` : ''}${mostReliable ? `<div><span>最高成功率</span><strong>${escapeHtml(mostReliable.model)} · ${mostReliable.success_rate.toFixed(1)}%</strong></div>` : ''}</div>`;
        const rows = summaries.map((summary, index) => `<tr><td><span class="benchmark-rank ${index < 3 ? 'top' : ''}">${index + 1}</span></td><td>${escapeHtml(summary.endpoint_name)}</td><td><strong>${escapeHtml(summary.model)}</strong></td><td><div class="benchmark-metric"><strong>${summary.success_rate.toFixed(1)}%</strong><div class="benchmark-meter"><span style="width:${summary.success_rate}%"></span></div></div></td><td>${summary.median_ttft_ms ?? '-'}</td><td>${summary.median_duration_ms ?? '-'}</td><td>${summary.average_total_tokens ?? '-'}</td><td>${summary.average_score != null ? `<span class="benchmark-score">${summary.average_score.toFixed(1)}</span>` : '-'}</td></tr>`).join('');
        const groups = data.run.cases.map(testCase => {
            const prompt = (Array.isArray(testCase.messages) ? testCase.messages : []).map(message => message.content).filter(Boolean).join('\n\n');
            const cards = data.run.attempts.filter(attempt => attempt.case_id === testCase.id).map(attempt => {
                const judge = data.run.judge_results.find(result => result.attempt_id === attempt.id);
                const judgeStatus = benchmarkJudgeStatus(judge);
                const score = judge?.score != null ? judge.score.toFixed(1) : '-';
                const judgeDetail = judge ? `<section class="benchmark-card-section"><h6>自动评审</h6><div class="benchmark-judge ${judgeStatus.className}"><span>${judgeStatus.text}</span>${judge.score != null ? `<strong>${score}</strong><span>准确性 ${judge.accuracy?.toFixed(1) ?? '-'} · 完整性 ${judge.completeness?.toFixed(1) ?? '-'} · 指令遵循 ${judge.instruction_following?.toFixed(1) ?? '-'} · 表达 ${judge.writing_quality?.toFixed(1) ?? '-'}</span>` : ''}${judge.reason ? `<p>${escapeHtml(judge.reason)}</p>` : ''}</div>${judge.raw_response ? `<details class="benchmark-judge-raw"><summary>查看评审原始响应</summary><pre class="replay-code">${escapeHtml(judge.raw_response)}</pre></details>` : ''}</section>` : '<section class="benchmark-card-section"><h6>自动评审</h6><p class="benchmark-empty">未生成评审结果</p></section>';
                return `<details class="benchmark-output-card"><summary class="benchmark-output-header"><div><strong>${escapeHtml(attempt.endpoint_name)}</strong><span>${escapeHtml(attempt.model)} · 第 ${attempt.attempt_number} 次</span></div><div class="benchmark-output-metrics"><span class="status-badge ${attempt.status === 'success' ? 'active' : 'disabled'}">${escapeHtml(attempt.status)}</span><span>${attempt.duration_ms}ms</span><strong class="benchmark-card-score">${score}</strong></div></summary><div class="benchmark-output-body"><section class="benchmark-card-section"><h6>模型输出</h6><pre class="replay-code">${escapeHtml(attempt.output || attempt.error_message || '')}</pre></section>${judgeDetail}</div></details>`;
            }).join('');
            return `<section class="benchmark-case-group"><h5>${escapeHtml(testCase.name)}</h5><pre class="benchmark-case-prompt">${escapeHtml(prompt)}</pre><div class="benchmark-output-grid">${cards || '<p class="benchmark-empty">暂无输出</p>'}</div></section>`;
        }).join('');
        detail.innerHTML = `${highlights}<div class="benchmark-summary-table"><div class="table-responsive"><table class="data-table"><thead><tr><th>排名</th><th>端点</th><th>模型</th><th>成功率</th><th>TTFT</th><th>总耗时</th><th>Token</th><th>评分</th></tr></thead><tbody>${rows}</tbody></table></div></div><div class="benchmark-attempts"><h4>样本输出</h4>${groups || '<p class="benchmark-empty">暂无输出</p>'}</div>`;
        showModal('benchmark-detail-modal');
    } catch { detail.innerHTML = '<p style="color:var(--danger);">加载评测结果失败</p>'; showModal('benchmark-detail-modal'); }
}

async function cancelModelBenchmark(id) {
    try { const response = await fetch(`${API_BASE}/model-benchmarks/${id}/cancel`, { method: 'POST' }); if (!response.ok) throw new Error(); showToast('评测任务已取消', 'success'); await loadModelBenchmarks(); } catch { showToast('取消评测任务失败', 'error'); }
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
                    <button class="btn btn-small ${api.replay_enabled ? 'btn-warning' : ''}" onclick="toggleApiReplay('${escapeAttr(api.id)}')">
                        ${api.replay_enabled ? '关闭回放' : '开启回放'}
                    </button>
                    <button class="btn btn-small" onclick="showReplayRecords('${escapeAttr(api.id)}')">回放记录 (${api.replay_record_count || 0})</button>
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

    const endpointSelect = document.getElementById('pool-test-endpoint-select');
    const modelsList = document.getElementById('pool-test-models-list');
    modelsList.innerHTML = '<p style="color: var(--text-secondary); padding: 16px; text-align: center;">加载中...</p>';

    showModal('pool-test-modal');

    try {
        const statsRes = await fetch(`${API_BASE}/stats`);
        const stats = await statsRes.json();
        const poolEndpoints = (stats.endpoints || []).filter(ep => (ep.pool_ids || []).includes(poolId));
        
        if (poolEndpoints.length > 0) {
            endpointSelect.innerHTML = poolEndpoints.map(ep =>
                `<option value="${escapeAttr(ep.id)}">${escapeHtml(ep.name)}</option>`
            ).join('');
            await loadPoolTestModelsForEndpoint(endpointSelect.value);
        } else {
            endpointSelect.innerHTML = '<option value="">无可用端点</option>';
            modelsList.innerHTML = '<p style="color: var(--text-tertiary); padding: 16px; text-align: center;">池中无端点，请先添加端点</p>';
        }
    } catch (e) {
        modelsList.innerHTML = `<p style="color: #f44336; padding: 16px;">加载失败: ${escapeHtml(e.message)}</p>`;
    }
}

async function loadPoolTestModelsForEndpoint(endpointId) {
    const modelsList = document.getElementById('pool-test-models-list');
    if (!endpointId) {
        modelsList.innerHTML = '<p style="color: var(--text-tertiary); padding: 16px; text-align: center;">请先选择端点</p>';
        currentPoolTestModels = [];
        return;
    }
    modelsList.innerHTML = '<p style="color: var(--text-secondary); padding: 16px; text-align: center;">加载模型列表...</p>';
    currentPoolTestModels = [];

    try {
        const epRes = await fetch(`${API_BASE}/endpoints/${encodeURIComponent(endpointId)}`);
        if (!epRes.ok) throw new Error('获取端点信息失败');
        const fullEp = await epRes.json();
        const config = fullEp.config;

        const modelsRes = await fetch(`${API_BASE}/endpoints/models`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
                name: config.name,
                url: config.url,
                api_type: config.api_type,
                api_key: config.api_key,
                token_limit: 1000,
                reset_policy: 'manual',
                enabled: true
            })
        });
        const result = await modelsRes.json();
        if (result.success && result.models) {
            currentPoolTestModels = result.models.map(m => typeof m === 'object' ? m.id : m);
            renderPoolTestModelList(currentPoolTestModels);
        } else {
            modelsList.innerHTML = '<p style="color: var(--text-tertiary); padding: 16px; text-align: center;">无法获取模型列表</p>';
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

    // 获取池中的端点列表
    let poolEndpoints = [];
    let pool = null;
    try {
        const statsRes = await fetch(`${API_BASE}/stats`);
        const stats = await statsRes.json();
        poolEndpoints = (stats.endpoints || []).filter(ep => (ep.pool_ids || []).includes(poolId));
        pool = (stats.pools || []).find(p => p.id === poolId);
    } catch (e) {
        showToast('获取池信息失败', 'error');
        return;
    }

    if (poolEndpoints.length === 0) {
        showToast('关联池中没有端点，请先添加端点', 'error');
        return;
    }

    // 用池中端点填充选择器
    populateTestEndpointSelectorFromList(poolEndpoints);
    showTestEndpointSelector(true);
    document.getElementById('models-modal-title').textContent = '选择测试模型';
    clearApiTestData();
    showModal('models-modal');

    // 保存上下文供切换时使用
    const modelsList = document.getElementById('models-list');
    if (modelsList) {
        modelsList.dataset.apiTestContext = JSON.stringify({ prefix: cleanPrefix, apiKey, apiType, baseUrl, poolId, pool });
    }

    // 加载默认（第一个）端点的模型列表
    await loadApiTestModels(cleanPrefix, apiKey, apiType, baseUrl, pool, poolEndpoints);
}

// 为 API 测试加载指定端点（由选择器选中）的模型列表
async function loadApiTestModelsForSelected() {
    const select = document.getElementById('test-endpoint-select');
    const modelsList = document.getElementById('models-list');
    if (!select || !select.value || !modelsList) return;

    const ctx = JSON.parse(modelsList.dataset.apiTestContext || '{}');
    if (!ctx.poolId) return;

    try {
        const statsRes = await fetch(`${API_BASE}/stats`);
        const stats = await statsRes.json();
        const poolEndpoints = (stats.endpoints || []).filter(ep => (ep.pool_ids || []).includes(ctx.poolId));
        const pool = (stats.pools || []).find(p => p.id === ctx.poolId);
        
        await loadApiTestModels(ctx.prefix, ctx.apiKey, ctx.apiType, ctx.baseUrl, pool, poolEndpoints);
    } catch (e) {
        if (modelsList) {
            modelsList.innerHTML = `<p style="color: var(--danger); padding: 16px; text-align: center;">请求失败: ${escapeHtml(e.message)}</p>`;
        }
    }
}

// 为 API 测试加载模型列表
async function loadApiTestModels(prefix, apiKey, apiType, baseUrl, pool, poolEndpoints) {
    const modelsList = document.getElementById('models-list');
    const modelsModalFooter = document.getElementById('models-modal-footer');
    const select = document.getElementById('test-endpoint-select');

    if (modelsList) {
        modelsList.innerHTML = '<p style="color: var(--text-secondary); padding: 16px; text-align: center;">加载模型列表...</p>';
    }
    if (modelsModalFooter) {
        modelsModalFooter.style.display = 'none';
    }

    const selectedId = select ? select.value : null;
    let selectedEndpoint = poolEndpoints.find(ep => ep.id === selectedId);
    if (!selectedEndpoint) {
        // fallback to first endpoint
        selectedEndpoint = poolEndpoints[0];
        if (select) {
            select.value = selectedEndpoint.id;
        }
    }

    try {
        let models = [];
        let modelMappings = [];

        if (pool && pool.model_mode === 'mapping') {
            const epRes = await fetch(`${API_BASE}/endpoints/${selectedEndpoint.id}`);
            if (epRes.ok) {
                const fullEp = await epRes.json();
                const mappings = fullEp.config.model_mappings || [];
                models = mappings.map(m => m.client_model);
                modelMappings = mappings;
            }
        } else {
            const epRes = await fetch(`${API_BASE}/endpoints/${selectedEndpoint.id}`);
            if (!epRes.ok) throw new Error('获取端点信息失败');
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
            renderApiModelSelectionList(models, {
                prefix,
                api_key: apiKey,
                api_type: apiType,
                base_url: baseUrl,
                model_mappings: modelMappings,
                endpoint_id: selectedEndpoint.id
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

async function toggleApiReplay(id) {
    try {
        const res = await fetch(`${API_BASE}/exposed-apis/${id}/replay-toggle`, { method: 'POST' });
        if (!res.ok) throw new Error('切换失败');
        const api = await res.json();
        showToast(api.replay_enabled ? '已开启数据回放' : '已关闭数据回放', 'success');
        loadApisPage();
    } catch (e) {
        showToast('切换数据回放失败', 'error');
    }
}

async function showReplayRecords(apiId) {
    currentReplayApiId = apiId;
    const api = currentApis.find(item => item.id === apiId);
    document.getElementById('replay-modal-title').textContent = `数据回放记录${api ? ` - ${api.name}` : ''}`;
    showModal('replay-modal');
    await loadReplayRecords();
}

async function loadReplayRecords() {
    if (!currentReplayApiId) return;
    const container = document.getElementById('replay-records-list');
    container.innerHTML = '<p style="color: var(--text-tertiary); padding: 16px;">正在加载回放记录...</p>';
    try {
        const res = await fetch(`${API_BASE}/exposed-apis/${currentReplayApiId}/replay-records`);
        if (!res.ok) throw new Error('加载失败');
        const data = await res.json();
        renderReplayRecords(data.records || []);
    } catch (e) {
        container.innerHTML = '<p style="color: var(--danger); padding: 16px;">加载回放记录失败</p>';
    }
}

function extractStreamedText(body) {
    const parts = [];
    let recognized = false;

    for (const line of body.split(/\r?\n/)) {
        if (!line.startsWith('data:')) continue;
        const payload = line.slice(5).trim();
        if (!payload || payload === '[DONE]') continue;

        try {
            const event = JSON.parse(payload);
            const openaiDelta = event.choices?.map(choice => choice.delta?.content || '').join('');
            const responsesDelta = event.type === 'response.output_text.delta' ? event.delta : '';
            const anthropicDelta = event.type === 'content_block_delta' ? event.delta?.text || '' : '';
            const text = openaiDelta || responsesDelta || anthropicDelta;
            if (text) {
                parts.push(text);
                recognized = true;
            }
        } catch (e) {
            // Incomplete SSE events remain available through the raw response fallback.
        }
    }

    return recognized ? parts.join('') : null;
}

function formatReplayBody(body) {
    if (!body) return '';
    const streamedText = extractStreamedText(body);
    if (streamedText !== null) return streamedText;
    try {
        return JSON.stringify(JSON.parse(body), null, 2);
    } catch (e) {
        return body;
    }
}

function renderReplayBody(title, body, truncated) {
    const truncation = truncated
        ? `<div class="replay-truncated-notice">内容超过配置阈值，仅显示前 ${currentReplayConfig.max_body_size_kb} KB</div>`
        : '';
    return `<section class="replay-body-section"><h4>${title}</h4>${truncation}<pre class="replay-code">${escapeHtml(formatReplayBody(body))}</pre></section>`;
}

function renderReplayRecords(records) {
    const container = document.getElementById('replay-records-list');
    document.getElementById('replay-record-count').textContent = `共 ${records.length} 条记录`;
    if (records.length === 0) {
        container.innerHTML = '<p style="color: var(--text-tertiary); padding: 16px; text-align: center;">暂无回放记录</p>';
        return;
    }
    container.innerHTML = records.slice().reverse().map(record => {
        const isError = record.status === 'error';
        const statusColor = isError ? 'var(--danger)' : 'var(--success)';
        const truncation = record.request_truncated || record.response_truncated
            ? '<span class="status-badge replay-badge">已截断</span>' : '';
        const error = record.error_message
            ? `<div style="color: var(--danger); font-size: 0.8125rem; margin-bottom: 10px;">${escapeHtml(record.error_message)}</div>` : '';
        return `<article class="replay-record">
            <div class="replay-record-summary ${isError ? 'error' : ''}" onclick="toggleReplayRecord(this)">
                <span style="color:${statusColor}; font-weight:600;">${record.status_code}</span>
                <span>${escapeHtml(record.method)}</span>
                <span class="replay-record-path" title="${escapeAttr(record.path)}">${escapeHtml(record.path)}</span>
                ${truncation}
                <span>${record.duration_ms} ms</span>
                <span style="color:var(--text-tertiary);">${new Date(record.timestamp).toLocaleString()}</span>
            </div>
            <div class="replay-record-detail">
                ${error}
                ${renderReplayBody('请求体', record.request_body, record.request_truncated)}
                ${renderReplayBody('响应体', record.response_body, record.response_truncated)}
            </div>
        </article>`;
    }).join('');
}

function toggleReplayRecord(summary) {
    summary.parentElement.classList.toggle('open');
}

async function clearReplayRecords() {
    if (!currentReplayApiId || !confirm('确定清空该接口的全部回放记录吗？')) return;
    try {
        const res = await fetch(`${API_BASE}/exposed-apis/${currentReplayApiId}/replay-records`, { method: 'DELETE' });
        if (!res.ok) throw new Error('清空失败');
        showToast('回放记录已清空', 'success');
        await loadReplayRecords();
        loadApisPage();
    } catch (e) {
        showToast('清空回放记录失败', 'error');
    }
}

async function loadReplayConfig() {
    try {
        const res = await fetch(`${API_BASE}/replay-config`);
        if (!res.ok) throw new Error('加载失败');
        currentReplayConfig = await res.json();
        document.getElementById('replay-state-file-path').value = currentReplayConfig.state_file_path;
        document.getElementById('replay-max-records').value = currentReplayConfig.max_records_per_api;
        document.getElementById('replay-max-body-kb').value = currentReplayConfig.max_body_size_kb;
    } catch (e) {
        showToast('加载回放设置失败', 'error');
    }
}

async function saveReplayConfig(event) {
    event.preventDefault();
    const data = {
        state_file_path: document.getElementById('replay-state-file-path').value.trim(),
        max_records_per_api: Number(document.getElementById('replay-max-records').value),
        max_body_size_kb: Number(document.getElementById('replay-max-body-kb').value),
    };
    try {
        const res = await fetch(`${API_BASE}/replay-config`, {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(data),
        });
        const response = await res.json();
        if (!res.ok) throw new Error(response.error?.message || '保存失败');
        currentReplayConfig = response;
        showToast('回放设置已保存', 'success');
    } catch (e) {
        showToast(e.message || '保存回放设置失败', 'error');
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
    document.getElementById('pool-retry-mode').value = pool.retry_mode || 'pool';
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

// 检查名称是否重复（前端即时校验，排除当前编辑的实体）
function checkDuplicateName(name, items, currentId, warningId) {
    const warning = document.getElementById(warningId);
    if (!warning) return true;
    if (!name || !name.trim()) {
        warning.style.display = 'none';
        return true;
    }
    const trimmed = name.trim();
    const isDuplicate = items.some(item => item.id !== currentId && item.name === trimmed);
    if (isDuplicate) {
        warning.textContent = `名称"${trimmed}"已存在，请使用其他名称`;
        warning.style.display = 'block';
    } else {
        warning.style.display = 'none';
    }
    return !isDuplicate;
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
        tbody.innerHTML = '<tr><td colspan="13" style="text-align: center; color: var(--text-secondary);">暂无调用日志</td></tr>';
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
                <td>${log.input_tokens !== undefined && log.input_tokens !== null ? formatNumber(log.input_tokens) : '-'}</td>
                <td>${log.output_tokens !== undefined && log.output_tokens !== null ? formatNumber(log.output_tokens) : '-'}</td>
                <td>${log.total_tokens !== undefined && log.total_tokens !== null ? formatNumber(log.total_tokens) : '-'}</td>
                <td>${errorMsg}</td>
            </tr>
        `;
    }).join('');
}

// ========== 延迟排行榜 ==========

async function loadLatencyLeaderboard() {
    const tbody = document.getElementById('latency-leaderboard-body');
    if (!tbody) return;

    tbody.innerHTML = '<tr><td colspan="11" style="text-align: center; color: var(--text-secondary);">加载中...</td></tr>';

    try {
        const res = await fetch(`${API_BASE}/latency-leaderboard`);
        if (!res.ok) {
            throw new Error(`HTTP ${res.status}`);
        }
        const data = await res.json();
        renderLatencyLeaderboard(data.leaderboard || []);
    } catch (e) {
        console.error('加载延迟排行榜失败:', e);
        tbody.innerHTML = `<tr><td colspan="11" style="text-align: center; color: var(--danger);">加载失败: ${escapeHtml(e.message)}</td></tr>`;
    }
}

function renderLatencyLeaderboard(stats) {
    const tbody = document.getElementById('latency-leaderboard-body');
    if (!tbody) return;

    if (!stats || stats.length === 0) {
        tbody.innerHTML = '<tr><td colspan="11" style="text-align: center; color: var(--text-secondary);">暂无延迟数据</td></tr>';
        return;
    }

    tbody.innerHTML = stats.map((item, index) => {
        const rank = index + 1;
        const rankClass = rank === 1 ? 'rank-1' : rank === 2 ? 'rank-2' : rank === 3 ? 'rank-3' : '';
        const enabledText = item.enabled ? '启用' : '禁用';
        const enabledClass = item.enabled ? 'status-active' : 'status-disabled';
        const formatMs = (ms) => ms > 0 ? `${ms}ms` : '-';
        const errorRate = item.error_rate !== undefined ? `${item.error_rate.toFixed(2)}%` : '-';
        const errorClass = item.error_rate >= 50 ? 'status-error' : item.error_rate >= 20 ? 'status-warning' : 'status-success';
        return `
            <tr>
                <td><span class="latency-rank ${rankClass}">${rank}</span></td>
                <td>${escapeHtml(item.endpoint_name)}</td>
                <td><span class="${enabledClass}">${enabledText}</span></td>
                <td><span class="${errorClass}">${errorRate}</span></td>
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

// ========== 技能仓库 ==========

const defaultSkillSources = [
    { id: 'github', name: 'GitHub', source_type: 'github', url: 'https://api.github.com', enabled: true, last_status: null, last_checked_at: null },
    { id: 'skillhub', name: 'SkillHub', source_type: 'skillhub', url: 'https://api.skillhub.cn', enabled: true, last_status: null, last_checked_at: null },
];

async function loadSkillRepository() {
    await Promise.all([loadLocalSkills(), loadSkillSources()]);
}

function switchSkillView(view) {
    document.querySelectorAll('.skill-tab').forEach(button => button.classList.toggle('active', button.dataset.skillView === view));
    document.querySelectorAll('.skill-view').forEach(panel => {
        panel.style.display = panel.id === `skill-${view}-view` ? '' : 'none';
    });
    if (view === 'sources') loadSkillSources();
}

async function readSkillApiError(response) {
    const body = await response.text();
    try { return JSON.parse(body).error?.message || JSON.parse(body).message || body; } catch { return body || `HTTP ${response.status}`; }
}

async function loadLocalSkills() {
    const container = document.getElementById('skill-local-list');
    if (!container) return;
    container.innerHTML = '<p class="skill-empty">正在读取本地技能...</p>';
    try {
        const response = await fetch(`${API_BASE}/skills`);
        if (!response.ok) throw new Error(await readSkillApiError(response));
        renderLocalSkills(await response.json());
    } catch (error) {
        container.innerHTML = `<p class="skill-empty skill-error">加载失败: ${escapeHtml(error.message)}</p>`;
    }
}

function renderLocalSkills(skills) {
    const container = document.getElementById('skill-local-list');
    if (!skills.length) {
        container.innerHTML = '<div class="skill-empty-panel"><strong>本地仓库还没有技能包</strong><span>上传包含根目录 SKILL.md 的 ZIP 文件，或从联网搜索中导入。</span></div>';
        return;
    }
    container.innerHTML = skills.map(skill => {
        const source = skill.source?.url || '本地上传';
        const valid = skill.validation_status === 'valid';
        return `<article class="skill-card">
            <div class="skill-card-top"><span class="skill-status ${valid ? 'valid' : 'invalid'}">${valid ? '有效' : '需处理'}</span><code>${escapeHtml(skill.directory_name)}</code></div>
            <h3>${escapeHtml(skill.name)}</h3>
            <p>${escapeHtml(skill.description || skill.validation_message || '未提供描述')}</p>
            <div class="skill-card-meta"><span>${skill.file_count} 个文件</span><span title="${escapeAttr(source)}">${escapeHtml(source)}</span></div>
            <div class="skill-card-actions"><button class="btn btn-small" type="button" onclick="openSkillDetails('${escapeAttr(skill.id)}')">查看详情</button><button class="btn btn-small btn-danger" type="button" onclick="confirmSkillDelete('${escapeAttr(skill.directory_name)}')">删除</button></div>
        </article>`;
    }).join('');
}

async function openSkillDetails(id) {
    try {
        const response = await fetch(`${API_BASE}/skills/${encodeURIComponent(id)}`);
        if (!response.ok) throw new Error(await readSkillApiError(response));
        const { skill, skill_md: skillMd, files } = await response.json();
        document.getElementById('skill-modal-title').textContent = skill.name;
        document.getElementById('skill-modal-body').innerHTML = `<div class="skill-detail-meta"><span class="skill-status ${skill.validation_status === 'valid' ? 'valid' : 'invalid'}">${escapeHtml(skill.validation_status)}</span><code>${escapeHtml(skill.directory_name)}</code><span>${skill.file_count} 个文件</span></div><p class="skill-detail-description">${escapeHtml(skill.description || '未提供描述')}</p><h3>SKILL.md</h3><pre class="skill-code">${escapeHtml(skillMd)}</pre><h3>文件清单</h3><ul class="skill-file-list">${files.map(file => `<li><code>${escapeHtml(file)}</code></li>`).join('')}</ul>`;
        document.getElementById('skill-modal-actions').innerHTML = `<button class="btn btn-secondary" type="button" onclick="hideModal('skill-modal')">关闭</button><button class="btn btn-danger" type="button" onclick="confirmSkillDelete('${escapeAttr(skill.directory_name)}')">删除技能</button>`;
        showModal('skill-modal');
    } catch (error) { showToast(`无法读取技能详情: ${error.message}`, 'error'); }
}

async function previewSkillUpload(event) {
    const file = event.target.files?.[0];
    event.target.value = '';
    if (!file) return;
    try {
        const response = await fetch(`${API_BASE}/skills/upload-preview`, { method: 'POST', headers: { 'Content-Type': 'application/zip' }, body: file });
        if (!response.ok) throw new Error(await readSkillApiError(response));
        showSkillImportPreview(await response.json(), '上传技能包');
    } catch (error) { showToast(`上传预览失败: ${error.message}`, 'error'); }
}

function showSkillImportPreview(preview, title) {
    document.getElementById('skill-modal-title').textContent = title;
    const conflictText = preview.conflict ? '<p class="skill-conflict">本地已存在同名目录。确认后将替换现有技能包。</p>' : '';
    document.getElementById('skill-modal-body').innerHTML = `<div class="skill-preview-summary"><strong>${escapeHtml(preview.target_directory_name)}</strong><span>${preview.files.length} 个文件</span><span>有效至 ${escapeHtml(formatDateTime(preview.expires_at))}</span></div>${conflictText}<h3>文件清单</h3><ul class="skill-file-list">${preview.files.map(file => `<li><code>${escapeHtml(file)}</code></li>`).join('')}</ul>`;
    const importAction = preview.conflict ? `confirmSkillImport('${escapeAttr(preview.id)}', true)` : `confirmSkillImport('${escapeAttr(preview.id)}', false)`;
    document.getElementById('skill-modal-actions').innerHTML = `<button class="btn btn-secondary" type="button" onclick="hideModal('skill-modal')">取消</button><button class="btn ${preview.conflict ? 'btn-danger' : 'btn-primary'}" type="button" onclick="${importAction}">${preview.conflict ? '替换本地版本' : '确认导入'}</button>`;
    showModal('skill-modal');
}

async function confirmSkillImport(previewId, replace) {
    try {
        const response = await fetch(`${API_BASE}/skills/import`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ preview_id: previewId, replace }) });
        if (!response.ok) throw new Error(await readSkillApiError(response));
        hideModal('skill-modal');
        showToast(replace ? '技能包已替换' : '技能包已导入', 'success');
        await loadLocalSkills();
        switchSkillView('local');
    } catch (error) { showToast(`导入失败: ${error.message}`, 'error'); }
}

async function confirmSkillDelete(directoryName) {
    const confirmation = prompt(`输入技能目录名“${directoryName}”以确认删除：`);
    if (confirmation === null) return;
    if (confirmation !== directoryName) { showToast('确认内容与技能目录不一致', 'error'); return; }
    try {
        const response = await fetch(`${API_BASE}/skills/${encodeURIComponent(directoryName)}`, { method: 'DELETE', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ confirmation }) });
        if (!response.ok) throw new Error(await readSkillApiError(response));
        hideModal('skill-modal');
        showToast('技能包已删除', 'success');
        await loadLocalSkills();
    } catch (error) { showToast(`删除失败: ${error.message}`, 'error'); }
}

async function searchSkills(event) {
    event.preventDefault();
    const keyword = document.getElementById('skill-search-keyword').value.trim();
    const results = document.getElementById('skill-search-results');
    const statuses = document.getElementById('skill-source-status');
    if (!keyword) return;
    results.innerHTML = '<p class="skill-empty">正在搜索公开来源...</p>';
    statuses.innerHTML = '';
    try {
        const response = await fetch(`${API_BASE}/skill-sources/search?keyword=${encodeURIComponent(keyword)}`);
        if (!response.ok) throw new Error(await readSkillApiError(response));
        const payload = await response.json();
        currentSkillSources = payload.sources || currentSkillSources;
        statuses.innerHTML = (payload.outcomes || []).map(outcome => `<span class="source-status ${outcome.error ? 'error' : 'available'}">${escapeHtml(outcome.source_id)}: ${escapeHtml(outcome.error || `${outcome.results.length} 个结果`)}</span>`).join('');
        const matches = (payload.outcomes || []).flatMap(outcome => outcome.results || []);
        renderSkillSearchResults(matches);
    } catch (error) { results.innerHTML = `<p class="skill-empty skill-error">搜索失败: ${escapeHtml(error.message)}</p>`; }
}

function renderSkillSearchResults(results) {
    const container = document.getElementById('skill-search-results');
    if (!results.length) { container.innerHTML = '<div class="skill-empty-panel"><strong>没有找到匹配的公开技能</strong><span>调整关键词，或检查来源设置中的启用状态。</span></div>'; return; }
    container.innerHTML = results.map(result => `<article class="skill-result-card">
        <div><div class="skill-result-heading"><h3>${escapeHtml(result.name)}</h3><span>${escapeHtml(result.source_id)}</span></div><p>${escapeHtml(result.description || '未提供描述')}</p><div class="skill-card-meta"><span>${escapeHtml(result.author || '未知作者')}</span><span>${result.popularity == null ? '无热度数据' : `热度 ${formatNumber(result.popularity)}`}</span><span>${escapeHtml(result.license || '许可证未知')}</span></div></div>
        <div class="skill-result-actions"><a href="${escapeAttr(result.source_url)}" target="_blank" rel="noopener noreferrer" class="btn btn-small">来源</a><button class="btn btn-primary btn-small" type="button" onclick="previewRemoteSkill('${escapeAttr(result.source_id)}', '${escapeAttr(result.download_locator)}', '${escapeAttr(result.version || '')}')">预览导入</button></div>
    </article>`).join('');
}

async function previewRemoteSkill(sourceId, archiveUrl, version) {
    try {
        const response = await fetch(`${API_BASE}/skill-sources/preview`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ source_id: sourceId, archive_url: archiveUrl, version: version || null }) });
        if (!response.ok) throw new Error(await readSkillApiError(response));
        showSkillImportPreview(await response.json(), '公开技能预览');
    } catch (error) { showToast(`远端预览失败: ${error.message}`, 'error'); }
}

async function loadSkillSources() {
    const container = document.getElementById('skill-source-list');
    if (!container) return;
    container.innerHTML = '<p class="skill-empty">正在读取来源设置...</p>';
    try {
        const response = await fetch(`${API_BASE}/skill-sources`);
        if (!response.ok) throw new Error(await readSkillApiError(response));
        currentSkillSources = await response.json();
        renderSkillSources();
    } catch (error) { container.innerHTML = `<p class="skill-empty skill-error">加载失败: ${escapeHtml(error.message)}</p>`; }
}

function renderSkillSources() {
    const container = document.getElementById('skill-source-list');
    if (!currentSkillSources.length) {
        container.innerHTML = '<div class="skill-empty-panel"><strong>尚未配置公开来源</strong><span>添加 GitHub、SkillHub 预置来源后即可开始联网搜索。</span><button class="btn btn-secondary btn-small" type="button" onclick="restoreDefaultSkillSources()">添加预置来源</button></div>';
        return;
    }
    container.innerHTML = currentSkillSources.map((source, index) => `<article class="skill-source-card" data-source-index="${index}">
        <div class="skill-source-card-title"><input class="source-enabled" type="checkbox" ${source.enabled ? 'checked' : ''} aria-label="启用 ${escapeAttr(source.name)}"><strong>${escapeHtml(source.name)}</strong><span class="source-status ${source.last_status && source.last_status !== 'available' ? 'error' : 'available'}">${escapeHtml(source.last_status || '未检测')}</span></div>
        <div class="skill-source-fields"><label>名称<input class="source-name" value="${escapeAttr(source.name)}"></label><label>类型<select class="source-type" ${source.source_type !== 'custom_index' ? 'disabled' : ''}><option value="custom_index" selected>自定义公开索引</option></select></label><label>HTTPS 地址<input class="source-url" type="url" value="${escapeAttr(source.url)}"></label></div>
        ${source.source_type === 'custom_index' ? `<button class="btn btn-danger btn-small" type="button" onclick="removeSkillSource(${index})">移除</button>` : ''}
    </article>`).join('');
}

function restoreDefaultSkillSources() { currentSkillSources = defaultSkillSources.map(source => ({ ...source })); renderSkillSources(); }
function addCustomSkillSource() {
    currentSkillSources.push({ id: `custom-${Date.now()}`, name: '自定义公开索引', source_type: 'custom_index', url: '', enabled: true, last_status: null, last_checked_at: null });
    renderSkillSources();
}
function removeSkillSource(index) { currentSkillSources.splice(index, 1); renderSkillSources(); }

async function saveSkillSources() {
    const cards = [...document.querySelectorAll('.skill-source-card')];
    const sources = cards.map((card, index) => ({
        ...currentSkillSources[index],
        enabled: card.querySelector('.source-enabled').checked,
        name: card.querySelector('.source-name').value.trim(),
        url: card.querySelector('.source-url').value.trim(),
    }));
    try {
        const response = await fetch(`${API_BASE}/skill-sources`, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(sources) });
        if (!response.ok) throw new Error(await readSkillApiError(response));
        currentSkillSources = await response.json();
        renderSkillSources();
        showToast('来源设置已保存', 'success');
    } catch (error) { showToast(`保存失败: ${error.message}`, 'error'); }
}
