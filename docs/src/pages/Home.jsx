import React from 'react';
import { motion } from 'framer-motion';
import { Terminal, Shield, Cpu, Zap, Box, Activity, ChevronRight, Globe, Code, Layers, Github } from 'lucide-react';
import SEO from '../components/SEO';

export default function Home() {
    return (
        <div className="w-full">
            <SEO
                title="Home"
                description="Ultraclaw AI is the world's most hyper-optimized native Rust autonomous agent framework. Zero-overhead execution across Linux, Windows, macOS, and Android."
            />

            {/* HERO SECTION */}
            <section className="relative pt-32 pb-40 flex flex-col items-center justify-center min-h-[90vh] text-center px-4 overflow-hidden">
                {/* Background ambient orbs */}
                <div className="absolute top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2 w-[800px] h-[800px] bg-cyan-600/10 rounded-full blur-[150px] pointer-events-none -z-10 animate-pulse"></div>
                <div className="absolute top-1/4 left-1/4 w-[400px] h-[400px] bg-indigo-600/10 rounded-full blur-[100px] pointer-events-none -z-10"></div>
                <div className="absolute bottom-1/4 right-1/4 w-[500px] h-[500px] bg-rose-600/10 rounded-full blur-[120px] pointer-events-none -z-10"></div>

                <motion.div initial={{ opacity: 0, y: 30 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: 0.8 }} className="z-10 flex flex-col items-center">

                    <motion.div
                        initial={{ scale: 0.8, opacity: 0 }}
                        animate={{ scale: 1, opacity: 1 }}
                        transition={{ delay: 0.2, type: "spring", stiffness: 200 }}
                        className="mb-8 relative group cursor-default"
                    >
                        <div className="absolute -inset-1 bg-gradient-to-r from-cyan-400 to-indigo-500 rounded-[2rem] blur opacity-25 group-hover:opacity-75 transition duration-1000 group-hover:duration-200"></div>
                        <div className="relative w-32 h-32 rounded-[2rem] overflow-hidden bg-black border border-white/10 ring-1 ring-white/20 shadow-2xl">
                            <img src={`${import.meta.env.BASE_URL}logo.png`} alt="Ultraclaw Master Logo" className="w-full h-full object-cover scale-110 group-hover:scale-100 transition-transform duration-700" />
                        </div>
                    </motion.div>

                    <motion.div
                        initial={{ opacity: 0, y: 20 }}
                        animate={{ opacity: 1, y: 0 }}
                        transition={{ delay: 0.4 }}
                        className="inline-flex items-center gap-2 px-4 py-2 rounded-full bg-white/5 border border-white/10 text-gray-300 text-sm font-medium mb-8 backdrop-blur-sm"
                    >
                        <span className="w-2 h-2 rounded-full bg-emerald-400 animate-ping"></span>
                        <span className="w-2 h-2 rounded-full bg-emerald-500 absolute"></span>
                        <span className="pl-2">Ultraclaw v1.0.0 is now live</span>
                    </motion.div>

                    <h1 className="text-6xl md:text-8xl lg:text-9xl font-black mb-6 tracking-tighter leading-tight">
                        <span className="text-gradient hover:drop-shadow-[0_0_30px_rgba(255,255,255,0.8)] transition-all duration-700">Ultraclaw</span><br />
                        <span className="text-4xl md:text-6xl text-white/90">Autonomous. Native. Fast.</span>
                    </h1>

                    <p className="text-xl md:text-2xl text-gray-400 font-medium max-w-4xl mb-12 leading-relaxed">
                        The world's most hyper-optimized <span className="text-white font-bold">Native Rust</span> AI agent framework.
                        We bypassed Python's 10x overhead completely to mount the Agent Singularity directly onto the OS Kernel.
                    </p>

                    <div className="flex flex-col sm:flex-row gap-6">
                        <a href="https://github.com/brainRottedCoder/Ultraclaw" target="_blank" rel="noreferrer" className="px-8 py-4 bg-white text-black font-bold rounded-xl hover:bg-gray-200 hover:scale-105 transition-all shadow-[0_0_40px_rgba(255,255,255,0.2)] flex items-center justify-center gap-3 text-lg group">
                            <Github className="w-5 h-5" /> View on GitHub
                        </a>
                        <a href="/Ultraclaw/downloads" className="px-8 py-4 bg-black/50 text-white font-bold border border-white/20 rounded-xl hover:bg-white/10 hover:border-cyan-400/50 hover:scale-105 transition-all backdrop-blur-md flex items-center justify-center gap-3 text-lg group">
                            <Box className="w-5 h-5 group-hover:text-cyan-400 transition-colors" /> Download Binaries
                        </a>
                    </div>
                </motion.div>
            </section>

            {/* TERMINAL UI SECTION */}
            <section className="py-24 px-6 md:px-12 max-w-7xl mx-auto border-t border-white/5 relative z-10">
                <div className="flex flex-col lg:flex-row items-center gap-16">
                    <div className="lg:w-1/2">
                        <h2 className="text-4xl md:text-5xl font-bold mb-6">A Terminal Interface<br />That Feels <span className="text-cyan-400 italic">Alive.</span></h2>
                        <p className="text-xl text-gray-400 mb-8 leading-relaxed">
                            Stop staring at scrolling Python standard output. Ultraclaw ships with a gorgeously complex, keyboard-driven Rust Ratatui visual interface natively compiled to `x86_64`. Monitor LLM heartbeat rates and inference streams entirely locally.
                        </p>
                        <ul className="space-y-4 mb-8">
                            {[
                                "Native Crossterm Keyboard Bounding",
                                "Zero-Latency Async Tokio Event Loops",
                                "Real-time Swarm and Node Routing Displays",
                                "Llama.cpp memory-mapped inference statistics"
                            ].map((item, i) => (
                                <li key={i} className="flex items-center gap-3 text-gray-300 font-medium">
                                    <div className="w-6 h-6 rounded-full bg-cyan-500/10 flex items-center justify-center border border-cyan-500/20">
                                        <ChevronRight className="w-4 h-4 text-cyan-400" />
                                    </div>
                                    {item}
                                </li>
                            ))}
                        </ul>
                    </div>

                    <div className="lg:w-1/2 w-full">
                        <motion.div
                            initial={{ opacity: 0, x: 50 }}
                            whileInView={{ opacity: 1, x: 0 }}
                            viewport={{ once: true }}
                            transition={{ duration: 0.8 }}
                            className="w-full glass-panel rounded-2xl border border-indigo-500/30 shadow-[0_0_50px_rgba(99,102,241,0.15)] overflow-hidden flex flex-col"
                        >
                            <div className="h-10 bg-black/80 border-b border-white/10 flex items-center px-4 gap-2">
                                <div className="w-3 h-3 rounded-full bg-rose-500"></div>
                                <div className="w-3 h-3 rounded-full bg-yellow-500"></div>
                                <div className="w-3 h-3 rounded-full bg-emerald-500"></div>
                                <div className="mx-auto text-xs font-mono text-gray-500">ultraclaw --tui</div>
                            </div>
                            <div className="p-6 font-mono text-sm text-green-400 h-[350px] overflow-y-auto bg-[#0a0a0a]">
                                <pre className="text-indigo-400 font-bold mb-4">{`
   __  __ __                           ___                 
  / / / // /_ _____ ____ _ _____ / /_ ____ _ __ __ __
 / /_/ // // // __// __ \`// ___// / // __ \`//| |/ |/ /
 \`____//_//_//_/   \\__,_/ \\___//_//_\\__,_/ |__/|__/ 
                                                        
`}</pre>
                                <p className="text-gray-400">[SYSTEM] Mounting Linux Landlock Sandbox... <span className="text-emerald-400">OK</span></p>
                                <p className="text-gray-400">[SYSTEM] Injecting 82 Skills into LLM Matrix... <span className="text-emerald-400">OK</span></p>
                                <p className="text-gray-400">[NETWORK] Starting Axum Local Host Port:3030... <span className="text-emerald-400">OK</span></p>
                                <br />
                                <div className="border border-white/10 p-4 rounded-xl bg-white/5">
                                    <p className="text-yellow-400 flex items-center gap-2">► Awaiting Agent Command Payload...</p>
                                    <div className="mt-2 h-2 w-1/3 bg-white/10 rounded-full overflow-hidden">
                                        <div className="h-full bg-cyan-400 animate-pulse w-full"></div>
                                    </div>
                                </div>
                            </div>
                        </motion.div>
                    </div>
                </div>
            </section>

            {/* FEATURE GRID */}
            <section className="py-32 px-6 md:px-12 max-w-7xl mx-auto border-t border-white/5 relative">
                <div className="absolute top-1/2 left-0 w-[400px] h-[400px] bg-indigo-600/5 rounded-full blur-[100px] pointer-events-none -z-10"></div>

                <div className="text-center mb-20">
                    <h2 className="text-4xl md:text-5xl font-bold mb-6">Architected for <span className="text-indigo-400">Absolute</span> Superiority</h2>
                    <p className="text-gray-400 text-xl max-w-3xl mx-auto leading-relaxed">Built from the absolute ground up, extracting native logic from the four great claw frameworks into one unified master executable.</p>
                </div>

                <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-8">
                    {[
                        { icon: Shield, color: "rose", title: "Linux Landlock Sandboxing", desc: "Native kernel-level system security prevents file deletion and bounds all network capabilities. Complete ephemeral safety." },
                        { icon: Code, color: "cyan", title: "React Native UI Hooks", desc: "Instantly hooks its logic into a stunning React interface wrapper. We don't do boring default interfaces." },
                        { icon: Zap, color: "yellow", title: "LLM Hot-Swappable", desc: "Supports OpenAI-V1 inference routing worldwide. Automatically fails over to local RAM-injected Llama CPU models on disconnect." },
                        { icon: Activity, color: "emerald", title: "Triple Interface UI", desc: "Ships naturally with a Rust Terminal application, an OS-level Desktop React app, and an Android APK all bundled." },
                        { icon: Globe, color: "blue", title: "18+ Massive Channels", desc: "Bridges Discord, Telegram, Slack, Teams, Signal, WhatsApp, and more absolutely natively via the asynchronous Axum Gateway." },
                        { icon: Layers, color: "purple", title: "80+ Omniscient Skills", desc: "File systems, web scraping, git merging, canvas generation, audio processing. Every skill compiled straight into the engine." }
                    ].map((feature, i) => (
                        <motion.div
                            key={i}
                            initial={{ opacity: 0, scale: 0.95, y: 30 }}
                            whileInView={{ opacity: 1, scale: 1, y: 0 }}
                            viewport={{ once: true }}
                            transition={{ delay: i * 0.1, type: "spring" }}
                            className="glass-panel p-8 rounded-3xl hover:bg-white/5 transition-all duration-300 border border-white/5 hover:-translate-y-2 group cursor-default relative overflow-hidden"
                        >
                            <div className="absolute inset-0 bg-gradient-to-br from-white/5 to-transparent opacity-0 group-hover:opacity-100 transition-opacity"></div>
                            <div className={`w-14 h-14 rounded-2xl flex items-center justify-center mb-6 transition-all shadow-lg border relative overflow-hidden group-hover:scale-110 duration-500`} style={{ backgroundColor: `color-mix(in srgb, var(--color-${feature.color}-500) 10%, transparent)`, borderColor: `color-mix(in srgb, var(--color-${feature.color}-500) 20%, transparent)` }}>
                                <feature.icon className={`w-7 h-7`} style={{ color: `var(--color-${feature.color}-400)` }} />
                            </div>
                            <h3 className="text-2xl font-bold mb-4">{feature.title}</h3>
                            <p className="text-gray-400 leading-relaxed text-lg">{feature.desc}</p>
                        </motion.div>
                    ))}
                </div>
            </section>
        </div>
    );
}
