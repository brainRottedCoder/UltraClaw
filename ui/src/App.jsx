import React, { useState } from 'react';
import { motion } from 'framer-motion';
import {
    Bot, CircuitBoard, Monitor, Database, Settings,
    MessageSquare, LayoutGrid, Terminal, AudioLines, Package, Search, ChevronRight, Activity, Command, ShieldAlert
} from 'lucide-react';

const SYSTEM_NODES = [
    "camera_snap", "camera_clip", "screen_record", "location_get",
    "system_run", "system_notify", "sessions_list",
    "sessions_history", "sessions_send", "sessions_spawn"
];

const CHANNELS = [
    "Slack", "Discord", "Telegram", "WhatsApp", "Google Chat", "Signal", "BlueBubbles", "iMessage",
    "Microsoft Teams", "Zalo", "ZaloPersonal", "WebChat", "Feishu", "QQ",
    "DingTalk", "LINE", "WeCom", "Nostr", "Twitch", "Mattermost"
];

const ULTRACLAW_SKILLS = [
    "1Password", "GitHub_Issues", "GitHub_PRs", "Notion", "Trello", "Asana",
    "Bear_Notes", "Apple_Notes", "Obsidian", "Jira", "Linear", "Docker_Manage",
    "Kubernetes_Pods", "AWS_EC2", "Tailscale_Serve", "Canvas_Render",
    "Git_Conflict_Resolve", "Apple_Health", "Sonos_Speaker", "HomeAssistant",
    "Hue_Lights", "Browser_Puppeteer", "Google_Drive", "Gmail", "Calendar",
    "Zoom_Launch", "Stripe_Charge", "Spotify_Play", "Figma_Read", "Slack_Post",
    // Truncated list for brevity, simulating 50+
    "Skill_31", "Skill_32", "Skill_33", "Skill_34", "Skill_35", "Skill_36",
    "Skill_37", "Skill_38", "Skill_39", "Skill_40", "Skill_41", "Skill_42",
    "Skill_43", "Skill_44", "Skill_45", "Skill_46", "Skill_47", "Skill_48",
    "Skill_49", "Skill_50"
];

export default function App() {
    const [activeTab, setActiveTab] = useState('canvas');
    const [search, setSearch] = useState('');
    const [toast, setToast] = useState(null);

    const triggerCapability = async (item) => {
        setToast(`Dispatching ${item} to Ultraclaw Core...`);

        try {
            const res = await fetch("http://127.0.0.1:3030/api/trigger", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify({ item })
            });
            const data = await res.json();
            setToast(data.output);
        } catch (err) {
            setToast(`Error: Ultraclaw Gateway offline (ensure rust core is running).`);
        }

        setTimeout(() => setToast(null), 5000);
    };

    // Reversing lists so they display "bottom to top" style as requested by user
    const nodesFiltered = [...SYSTEM_NODES].reverse().filter(s => s.toLowerCase().includes(search.toLowerCase()));
    const channelsFiltered = [...CHANNELS].reverse().filter(s => s.toLowerCase().includes(search.toLowerCase()));
    const skillsFiltered = [...ULTRACLAW_SKILLS].reverse().filter(s => s.toLowerCase().includes(search.toLowerCase()));

    return (
        <div className="h-screen w-full bg-zinc-950 flex overflow-hidden font-sans text-gray-200">

            {/* 1. Universal Capabilities Sidebar */}
            <motion.div
                initial={{ x: -300 }}
                animate={{ x: 0 }}
                className="w-80 h-full glass-panel border-r border-white/5 flex flex-col z-20"
            >
                <div className="p-6 border-b border-white/5 flex items-center space-x-3">
                    <div className="w-10 h-10 rounded-xl overflow-hidden shadow-lg shadow-indigo-500/20 bg-black">
                        <img src="/logo.png" alt="Ultraclaw Logo" className="w-full h-full object-cover" />
                    </div>
                    <div>
                        <h1 className="text-lg font-bold bg-clip-text text-transparent bg-gradient-to-r from-white to-gray-400">Ultraclaw</h1>
                        <p className="text-xs text-indigo-400 font-medium tracking-wider uppercase">Master Console</p>
                    </div>
                </div>

                {/* Global Search */}
                <div className="p-4">
                    <div className="relative">
                        <Search className="absolute left-3 top-2.5 h-4 w-4 text-gray-500" />
                        <input
                            type="text"
                            placeholder="Search all 80+ capabilities..."
                            value={search}
                            onChange={(e) => setSearch(e.target.value)}
                            className="w-full bg-white/5 border border-white/10 rounded-lg py-2 pl-9 pr-4 text-sm focus:outline-none focus:ring-2 focus:ring-indigo-500/50 transition-all placeholder-gray-600"
                        />
                    </div>
                </div>

                {/* The Massive List (Bottom to Top rendering) */}
                <div className="flex-1 overflow-y-auto px-3 pb-6 space-y-8 custom-scrollbar">

                    {/* OS Nodes */}
                    <div>
                        <div className="px-3 mb-2 flex items-center space-x-2 text-rose-400">
                            <Terminal className="h-4 w-4" />
                            <h2 className="text-xs font-bold uppercase tracking-widest">OS System Nodes (10)</h2>
                        </div>
                        <div className="space-y-1">
                            {nodesFiltered.map(node => (
                                <button onClick={() => triggerCapability(node)} key={node} className="w-full flex items-center justify-between px-3 py-2 rounded-lg hover:bg-white/5 transition-colors group">
                                    <span className="text-sm font-medium text-gray-300 group-hover:text-white">{node}</span>
                                    <ChevronRight className="h-3 w-3 text-gray-600 group-hover:text-rose-400 transition-colors" />
                                </button>
                            ))}
                        </div>
                    </div>

                    {/* Connectors */}
                    <div>
                        <div className="px-3 mb-2 flex items-center space-x-2 text-emerald-400">
                            <MessageSquare className="h-4 w-4" />
                            <h2 className="text-xs font-bold uppercase tracking-widest">Channels (18)</h2>
                        </div>
                        <div className="space-y-1">
                            {channelsFiltered.map(ch => (
                                <button onClick={() => triggerCapability(ch)} key={ch} className="w-full flex items-center justify-between px-3 py-2 rounded-lg hover:bg-white/5 transition-colors group">
                                    <span className="text-sm font-medium text-gray-300 group-hover:text-white">{ch}</span>
                                    <div className="h-2 w-2 rounded-full bg-emerald-500/50 group-hover:bg-emerald-400 shadow-[0_0_8px_rgba(52,211,153,0.5)]"></div>
                                </button>
                            ))}
                        </div>
                    </div>

                    {/* OpenClaw Skills */}
                    <div>
                        <div className="px-3 mb-2 flex items-center space-x-2 text-indigo-400">
                            <Package className="h-4 w-4" />
                            <h2 className="text-xs font-bold uppercase tracking-widest">Ultraclaw Skills (50+)</h2>
                        </div>
                        <div className="space-y-1">
                            {skillsFiltered.map(skill => (
                                <button onClick={() => triggerCapability(skill)} key={skill} className="w-full flex items-center justify-between px-3 py-2 rounded-lg hover:bg-white/5 transition-colors group">
                                    <span className="text-sm font-medium text-gray-300 group-hover:text-white">{skill}</span>
                                    <ChevronRight className="h-3 w-3 text-gray-600 group-hover:text-indigo-400 transition-colors" />
                                </button>
                            ))}
                        </div>
                    </div>

                </div>
            </motion.div>

            {/* 2. Main Workspace Area */}
            <div className="flex-1 flex flex-col relative">
                {/* Top Navbar */}
                <header className="h-16 glass-panel border-b border-white/5 flex items-center justify-between px-8 z-10">
                    <div className="flex space-x-1 bg-white/5 p-1 rounded-lg">
                        {['canvas', 'voice', 'swarm', 'landlock', 'settings'].map((tab) => (
                            <button
                                key={tab}
                                onClick={() => setActiveTab(tab)}
                                className={`px-4 py-1.5 rounded-md text-sm font-medium transition-all ${activeTab === tab ? 'bg-indigo-500 text-white shadow-lg' : 'text-gray-400 hover:text-white hover:bg-white/5'
                                    }`}
                            >
                                {tab === 'canvas' && <span className="flex items-center gap-2"><LayoutGrid className="w-4 h-4" /> Live Canvas</span>}
                                {tab === 'voice' && <span className="flex items-center gap-2"><AudioLines className="w-4 h-4" /> Voice Mode</span>}
                                {tab === 'swarm' && <span className="flex items-center gap-2"><CircuitBoard className="w-4 h-4" /> Swarms</span>}
                                {tab === 'landlock' && <span className="flex items-center gap-2 text-rose-400"><ShieldAlert className="w-4 h-4" /> Landlock</span>}
                                {tab === 'settings' && <span className="flex items-center gap-2 text-cyan-400"><Settings className="w-4 h-4" /> Neural Config</span>}
                            </button>
                        ))}
                    </div>

                    <div className="flex items-center space-x-4">
                        <div className="flex items-center space-x-2">
                            <span className="relative flex h-3 w-3">
                                <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-emerald-400 opacity-75"></span>
                                <span className="relative inline-flex rounded-full h-3 w-3 bg-emerald-500"></span>
                            </span>
                            <span className="text-xs font-medium text-emerald-400">Ultraclaw Core Connected</span>
                        </div>
                    </div>
                </header>

                {/* Dynamic Content Pane */}
                <div className="flex-1 p-8 overflow-y-auto relative">

                    {/* Background decoration */}
                    <div className="absolute top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2 w-[800px] h-[800px] bg-indigo-500/5 rounded-full blur-[120px] pointer-events-none -z-10"></div>

                    {activeTab === 'canvas' && (
                        <motion.div initial={{ opacity: 0, y: 20 }} animate={{ opacity: 1, y: 0 }} className="h-full flex gap-6">
                            <div className="flex-1 glass-panel rounded-2xl p-6 border border-white/10 flex flex-col">
                                <h2 className="text-lg font-bold mb-4 flex items-center gap-2"><MessageSquare className="w-5 h-5 text-indigo-400" /> Context Stream</h2>
                                <div className="flex-1 rounded-xl bg-black/50 border border-white/5 p-4 overflow-y-auto custom-scrollbar">
                                    <div className="space-y-4">
                                        <div className="bg-indigo-500/10 border border-indigo-500/20 p-4 rounded-xl max-w-[80%]">
                                            <p className="text-sm text-indigo-200">System booted payload successfully. Group context mounted. 53 Ultraclaw tools mapped into schema.</p>
                                        </div>
                                    </div>
                                </div>
                                <div className="mt-4 relative">
                                    <input type="text" placeholder="Command Ultraclaw..." className="w-full bg-white/5 border border-white/10 rounded-xl py-4 pl-4 pr-12 text-sm focus:outline-none focus:ring-2 focus:ring-indigo-500" />
                                    <button className="absolute right-2 top-2 p-2 bg-indigo-500 rounded-lg hover:bg-indigo-400 transition-colors">
                                        <Command className="w-4 h-4" />
                                    </button>
                                </div>
                            </div>
                            <div className="flex-1 glass-panel rounded-2xl p-6 border border-white/10 flex flex-col">
                                <h2 className="text-lg font-bold mb-4 flex items-center gap-2"><LayoutGrid className="w-5 h-5 text-purple-400" /> Live Canvas Output</h2>
                                <div className="flex-1 rounded-xl bg-zinc-900 border border-white/5 flex items-center justify-center p-8">
                                    <div className="text-center">
                                        <Monitor className="w-16 h-16 text-zinc-800 mx-auto mb-4" />
                                        <p className="text-zinc-500">Awaiting visual artifacts...</p>
                                    </div>
                                </div>
                            </div>
                        </motion.div>
                    )}

                    {activeTab === 'voice' && (
                        <motion.div initial={{ opacity: 0, scale: 0.95 }} animate={{ opacity: 1, scale: 1 }} className="h-full flex items-center justify-center">
                            <div className="text-center group cursor-pointer">
                                <div className="w-48 h-48 rounded-full bg-gradient-to-tr from-rose-500 to-indigo-600 flex items-center justify-center relative shadow-[0_0_50px_rgba(99,102,241,0.3)] group-hover:shadow-[0_0_80px_rgba(99,102,241,0.5)] transition-all duration-500">
                                    <div className="absolute inset-0 rounded-full animate-ping opacity-20 bg-white"></div>
                                    <AudioLines className="w-20 h-20 text-white" />
                                </div>
                                <h2 className="mt-8 text-2xl font-bold bg-clip-text text-transparent bg-gradient-to-r from-white to-gray-400">Listening to Environment</h2>
                                <p className="text-gray-500 mt-2">Whisper Engine Active • Hardware Mic Bound</p>
                            </div>
                        </motion.div>
                    )}

                    {activeTab === 'swarm' && (
                        <motion.div initial={{ opacity: 0, y: 20 }} animate={{ opacity: 1, y: 0 }} className="h-full">
                            <h2 className="text-2xl font-bold mb-6 flex items-center gap-3"><CircuitBoard className="w-6 h-6 text-emerald-400" /> Active Delegations (Agent Swarms)</h2>
                            <div className="grid grid-cols-3 gap-6">
                                {[1, 2, 3].map(i => (
                                    <div key={i} className="glass-panel p-6 rounded-2xl border border-white/10 relative overflow-hidden group">
                                        <div className="absolute top-0 right-0 w-32 h-32 bg-emerald-500/10 rounded-full blur-3xl group-hover:bg-emerald-500/20 transition-colors"></div>
                                        <div className="flex justify-between items-start mb-4">
                                            <div className="w-10 h-10 rounded-full bg-white/5 flex items-center justify-center border border-white/10">
                                                <Activity className="w-5 h-5 text-emerald-400" />
                                            </div>
                                            <span className="px-2 py-1 bg-emerald-500/10 text-emerald-400 text-xs rounded-full border border-emerald-500/20">Working</span>
                                        </div>
                                        <h3 className="text-lg font-bold mb-1">Sub-Agent {i}</h3>
                                        <p className="text-sm text-gray-400">Delegated task: Refactoring codebase components via nano-git resolution engine.</p>
                                        <div className="mt-6 w-full bg-white/5 h-1.5 rounded-full overflow-hidden">
                                            <div className="bg-gradient-to-r from-emerald-500 to-cyan-400 h-full w-2/3 animate-pulse"></div>
                                        </div>
                                    </div>
                                ))}
                            </div>
                        </motion.div>
                    )}

                    {activeTab === 'settings' && (
                        <motion.div initial={{ opacity: 0, y: 20 }} animate={{ opacity: 1, y: 0 }} className="h-full max-w-3xl mx-auto">
                            <h2 className="text-2xl font-bold mb-2 flex items-center gap-3"><Database className="w-6 h-6 text-cyan-400" /> Global LLM Integrations</h2>
                            <p className="text-gray-400 mb-8 border-b border-white/10 pb-6">Configure any local or cloud AI provider worldwide. The Rust core automatically routes OpenAI-compatible architectures dynamically.</p>

                            <div className="glass-panel p-8 rounded-2xl border border-white/10 relative overflow-hidden">
                                <form onSubmit={async (e) => {
                                    e.preventDefault();
                                    const formData = new FormData(e.target);
                                    const payload = Object.fromEntries(formData);

                                    setToast(`Binding Core to ${payload.provider}...`);
                                    try {
                                        const res = await fetch("http://127.0.0.1:3030/api/config", {
                                            method: "POST",
                                            headers: { "Content-Type": "application/json" },
                                            body: JSON.stringify(payload)
                                        });
                                        const data = await res.json();
                                        setToast(data.output);
                                    } catch (err) {
                                        setToast(`Error establishing connection to Master Core backend.`);
                                    }
                                    setTimeout(() => setToast(null), 5000);
                                }} className="space-y-6">
                                    <div className="grid grid-cols-2 gap-6">
                                        <div>
                                            <label className="block text-sm font-medium text-gray-400 mb-2">Provider Archetype</label>
                                            <select name="provider" className="w-full bg-black/50 border border-white/10 rounded-lg py-3 px-4 text-sm focus:ring-2 focus:ring-cyan-500 text-white outline-none cursor-pointer">
                                                <option value="OpenAI">OpenAI (GPT-4o)</option>
                                                <option value="Anthropic">Anthropic (Claude 3.5)</option>
                                                <option value="Google">Google (Gemini Pro)</option>
                                                <option value="Groq">Groq (LPU Inference)</option>
                                                <option value="DeepSeek">DeepSeek (V3 & R1)</option>
                                                <option value="Local_Ollama">Ollama (Local Host)</option>
                                                <option value="Local_LMStudio">LM Studio (Local Host)</option>
                                                <option value="Custom">Custom Server (vLLM, etc)</option>
                                            </select>
                                        </div>
                                        <div>
                                            <label className="block text-sm font-medium text-gray-400 mb-2">Model Name ID</label>
                                            <input name="model" type="text" defaultValue="llama3" placeholder="e.g. gpt-4o, claude-3-haiku, deepseek-chat" className="w-full bg-black/50 border border-white/10 rounded-lg py-3 px-4 text-sm focus:outline-none focus:ring-2 focus:ring-cyan-500 text-white" required />
                                        </div>
                                    </div>

                                    <div>
                                        <label className="block text-sm font-medium text-gray-400 mb-2">API Base URL <span className="text-xs text-gray-500 ml-2">(Standard OpenAI V1 compatibility)</span></label>
                                        <input name="base_url" type="url" defaultValue="http://localhost:11434/v1" placeholder="https://api.openai.com/v1" className="w-full bg-black/50 border border-white/10 rounded-lg py-3 px-4 text-sm focus:outline-none focus:ring-2 focus:ring-cyan-500 text-white font-mono" required />
                                    </div>

                                    <div>
                                        <label className="block text-sm font-medium text-gray-400 mb-2">Secret Inference Key <span className="text-xs text-gray-500 ml-2">(Never stored; sent strictly to RAM)</span></label>
                                        <input name="api_key" type="password" placeholder="sk-..." className="w-full bg-black/50 border border-white/10 rounded-lg py-3 px-4 text-sm focus:outline-none focus:ring-2 focus:ring-cyan-500 text-white font-mono" />
                                    </div>

                                    <div className="pt-4 border-t border-white/10 flex justify-end">
                                        <button type="submit" className="px-6 py-3 bg-cyan-500 hover:bg-cyan-400 text-black font-bold rounded-lg transition-colors flex items-center gap-2">
                                            <Database className="w-4 h-4" />
                                            Synchronize Core Config
                                        </button>
                                    </div>
                                </form>
                            </div>
                        </motion.div>
                    )}

                </div>
            </div>

            {/* Global Toast Notification Overlay */}
            {toast && (
                <motion.div
                    initial={{ opacity: 0, y: 50, x: '-50%' }}
                    animate={{ opacity: 1, y: 0, x: '-50%' }}
                    exit={{ opacity: 0, y: 50, x: '-50%' }}
                    className="fixed bottom-10 left-1/2 -translate-x-1/2 bg-indigo-500 text-white px-6 py-3 rounded-full shadow-2xl flex items-center space-x-3 z-50 border border-indigo-400"
                >
                    <Activity className="h-4 w-4 animate-spin-slow" />
                    <span className="text-sm font-medium">{toast}</span>
                </motion.div>
            )}

        </div>
    );
}
