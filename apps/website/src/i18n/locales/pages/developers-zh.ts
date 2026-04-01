export const zhDevelopers: Record<string, string> = {
	// Meta
	"dev.meta.title": "开发者专区 | Flow-Like",
	"dev.meta.description":
		"两种方式构建 Flow-Like 应用：使用现有节点可视化编排工作流，或使用 15+ 种语言编写自定义节点并编译为 WebAssembly。",

	// Hero
	"dev.hero.badge": "开源项目",
	"dev.hero.headline.prefix": "面向",
	"dev.hero.headline.highlight": "开发者",
	"dev.hero.description":
		"无论你是通过可视化方式构建工作流，还是用你偏好的语言编写自定义节点——Flow-Like 都为你提供两条清晰的路径来交付生产级自动化方案。",
	"dev.hero.cio.prefix": "正在寻找高管概述？",
	"dev.hero.cio.link": "CIO 专区 →",
	"dev.hero.card.workflows.title": "编排工作流",
	"dev.hero.card.workflows.description": "拖拽、连接、部署——无需编写代码",
	"dev.hero.card.nodes.title": "编写自定义节点",
	"dev.hero.card.nodes.description": "15+ 种语言，编译为 WebAssembly",
	"dev.hero.converge": "部署到生产环境",

	// Path Picker
	"dev.pathpicker.label": "选择你的路径",

	// Workflow Path
	"dev.workflow.badge": "路径二",
	"dev.workflow.headline": "可视化编排工作流",
	"dev.workflow.description":
		"使用可视化编辑器拖拽并连接预构建节点，创建强大的自动化流程。无需编写代码——只需连接、配置并部署。",
	"dev.workflow.feature.dragdrop.title": "拖拽式构建器",
	"dev.workflow.feature.dragdrop.description":
		"可视化画布让你通过连接节点来构建复杂的数据和自动化流水线。每个连接都会进行实时类型检查。",
	"dev.workflow.feature.catalog.title": "丰富的节点目录",
	"dev.workflow.feature.catalog.description":
		"数百个预构建节点，涵盖数据库、API、AI/ML、文件操作、消息通信等领域——随时可拖入你的工作流。",
	"dev.workflow.feature.templates.title": "模板与共享",
	"dev.workflow.feature.templates.description":
		"从模板起步，或发布你自己的作品。通过版本控制在团队间共享工作流，让所有人都能受益于经过验证的最佳实践。",
	"dev.workflow.feature.interfaces.title": "构建界面",
	"dev.workflow.feature.interfaces.description":
		"使用内置的界面编辑器创建自定义 UI。将任何工作流转化为团队可直接使用的交互式应用。",
	"dev.workflow.feature.vcs.title": "版本控制",
	"dev.workflow.feature.vcs.description":
		"每个工作流都可序列化和对比差异。将工作流存储在 Git 中，在 PR 中审查变更，并自信地回滚。",
	"dev.workflow.feature.typesafe.title": "类型安全执行",
	"dev.workflow.feature.typesafe.description":
		"输入和输出在编译时即被验证。在部署前而非生产环境中捕获类型不匹配问题。",
	"dev.workflow.howitworks": "工作原理",
	"dev.workflow.step1.title": "打开可视化画布",
	"dev.workflow.step1.description": "在桌面应用或 Web 工作台中",
	"dev.workflow.step2.title": "从目录中拖入节点",
	"dev.workflow.step2.description": "搜索、筛选或浏览分类",
	"dev.workflow.step3.title": "连接并配置",
	"dev.workflow.step3.description": "类型安全连线与实时验证",
	"dev.workflow.step4.title": "运行或部署",
	"dev.workflow.step4.description": "本地、自托管或云端",
	"dev.workflow.cta.docs": "阅读文档",
	"dev.workflow.cta.download": "下载 Flow-Like",

	// Custom Nodes Path
	"dev.nodes.divider": "或",
	"dev.nodes.badge": "路径一",
	"dev.nodes.headline": "编写自定义节点",
	"dev.nodes.description":
		"用你自己的逻辑扩展引擎。使用任何支持的语言编写节点——它会被编译为 WebAssembly，在沙箱中运行，并拥有对宿主 SDK 的完整访问权限（日志、存储、HTTP、AI 模型等）。",
	"dev.nodes.languages.title": "支持的语言",
	"dev.nodes.languages.description":
		"每种语言都配备项目模板和 SDK。选择一种即可开始构建。",
	"dev.nodes.languages.sdk": "完整 SDK",
	"dev.nodes.sdk.title": "宿主 SDK 能力",
	"dev.nodes.sdk.description":
		"每个节点通过 SDK 获得对以下平台 API 的沙箱化访问权限：",
	"dev.nodes.sdk.logging": "日志",
	"dev.nodes.sdk.logging.desc": "结构化日志输出",
	"dev.nodes.sdk.pins": "引脚",
	"dev.nodes.sdk.pins.desc": "读写节点 I/O",
	"dev.nodes.sdk.variables": "变量",
	"dev.nodes.sdk.variables.desc": "工作流级状态",
	"dev.nodes.sdk.cache": "缓存",
	"dev.nodes.sdk.cache.desc": "持久化 KV cache",
	"dev.nodes.sdk.metadata": "元数据",
	"dev.nodes.sdk.metadata.desc": "节点与工作流信息",
	"dev.nodes.sdk.streaming": "流式处理",
	"dev.nodes.sdk.streaming.desc": "流式传输大数据",
	"dev.nodes.sdk.storage": "存储",
	"dev.nodes.sdk.storage.desc": "文件与对象存储",
	"dev.nodes.sdk.ai": "AI 模型",
	"dev.nodes.sdk.ai.desc": "LLM 与嵌入调用",
	"dev.nodes.sdk.http": "HTTP",
	"dev.nodes.sdk.http.desc": "出站 HTTP 请求",
	"dev.nodes.sdk.auth": "认证",
	"dev.nodes.sdk.auth.desc": "密钥与凭据",
	"dev.nodes.feature.sandbox.title": "沙箱隔离，安全可靠",
	"dev.nodes.feature.sandbox.description":
		"节点在 WebAssembly 沙箱中运行，采用基于能力的权限模型。除非明确授权，否则无法访问文件系统或网络。",
	"dev.nodes.feature.test.title": "测试与迭代",
	"dev.nodes.feature.test.description":
		"每个模板都附带测试工具。编写测试用例，在本地运行，验证通过后再发布到目录。",
	"dev.nodes.feature.publish.title": "发布与版本管理",
	"dev.nodes.feature.publish.description":
		"为节点打包元数据，使用语义化版本控制，并可私有共享或发布到公共目录。",
	"dev.nodes.quickstart.title": "快速开始",
	"dev.nodes.quickstart.description":
		"打开 Flow-Like Studio，选择语言模板，开始构建你的自定义节点：",
	"dev.nodes.quickstart.step1":
		"1. 打开 Flow-Like Studio 并导航到节点开发者区域",
	"dev.nodes.quickstart.step2": "2. 从 15+ 种支持的语言中选择模板",
	"dev.nodes.quickstart.step3": "3. 实现你的节点逻辑并构建 WASM 二进制文件",
	"dev.nodes.quickstart.step4": "4. 直接从 Studio 发布到你的目录",
	"dev.nodes.cta.guide": "节点开发指南",
	"dev.nodes.cta.templates": "浏览所有模板",

	// CTA
	"dev.cta.headline": "准备好开始构建了吗？",
	"dev.cta.description":
		"无论你是可视化编排还是编写自定义节点——Flow-Like 是开源的、本地优先的，专为希望完全掌控自动化技术栈的开发者而打造。",
	"dev.cta.download": "下载 Flow-Like",
	"dev.cta.github": "在 GitHub 上查看",
};
