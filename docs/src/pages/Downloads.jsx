import React, { useEffect, useState } from 'react';
import { motion } from 'framer-motion';
import { DownloadCloud, Apple, Monitor, Smartphone, Terminal, Package, ArrowRight } from 'lucide-react';
import SEO from '../components/SEO';

export default function Downloads() {
    return (
        <div className="w-full max-w-7xl mx-auto px-6 md:px-12 py-20 relative">
            <SEO
                title="Download"
                description="Download the latest pre-compiled Native Rust Binaries of Ultraclaw for Windows, macOS, Linux, and Android. Built automatically by GitHub Actions."
                path="/downloads"
            />

            <div className="absolute top-0 right-1/4 w-[600px] h-[600px] bg-cyan-500/10 rounded-full blur-[120px] pointer-events-none -z-10"></div>

            <div className="text-center mb-16">
                <motion.div initial={{ opacity: 0, y: 20 }} animate={{ opacity: 1, y: 0 }} className="inline-flex items-center gap-2 px-4 py-2 rounded-full bg-cyan-500/10 border border-cyan-500/20 text-cyan-400 text-sm font-semibold mb-6">
                    <DownloadCloud className="w-4 h-4" /> Latest Pipeline Release Available
                </motion.div>

                <motion.h1 initial={{ opacity: 0, y: 20 }} animate={{ opacity: 1, y: 0 }} transition={{ delay: 0.1 }} className="text-5xl md:text-6xl font-black mb-6">
                    Native <span className="text-cyan-400">Payloads</span>
                </motion.h1>

                <motion.p initial={{ opacity: 0, y: 20 }} animate={{ opacity: 1, y: 0 }} transition={{ delay: 0.2 }} className="text-xl text-gray-400 max-w-2xl mx-auto">
                    Ultraclaw isn't a Python script you execute blindly. It's a hyper-optimized Rust binary compiled directly into machine code for your OS.
                </motion.p>
            </div>

            <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-6">

                {/* WINDOWS */}
                <motion.div initial={{ opacity: 0, y: 20 }} animate={{ opacity: 1, y: 0 }} transition={{ delay: 0.3 }} className="glass-panel p-8 rounded-2xl hover:-translate-y-2 transition-transform duration-300 border border-white/5 group flex flex-col justify-between h-full relative overflow-hidden">
                    <div className="absolute inset-0 bg-blue-500/5 opacity-0 group-hover:opacity-100 transition-opacity"></div>
                    <div>
                        <Monitor className="w-10 h-10 text-blue-400 mb-6" />
                        <h3 className="text-2xl font-bold mb-2">Windows</h3>
                        <p className="text-gray-400 text-sm mb-6">x86_64 Native Executable. Bundled with the Tauri React Desktop App.</p>
                    </div>
                    <a href="https://github.com/brainRottedCoder/Ultraclaw/releases/latest" data-os="windows" target="_blank" rel="noreferrer" className="w-full py-4 rounded-xl bg-white/5 hover:bg-blue-500 hover:text-white transition-colors border border-white/10 flex items-center justify-center gap-2 font-bold group-hover:border-transparent z-10">
                        Download .exe
                    </a>
                </motion.div>

                {/* MACOS */}
                <motion.div initial={{ opacity: 0, y: 20 }} animate={{ opacity: 1, y: 0 }} transition={{ delay: 0.4 }} className="glass-panel p-8 rounded-2xl hover:-translate-y-2 transition-transform duration-300 border border-white/5 group flex flex-col justify-between h-full relative overflow-hidden">
                    <div className="absolute inset-0 bg-gray-500/5 opacity-0 group-hover:opacity-100 transition-opacity"></div>
                    <div>
                        <Apple className="w-10 h-10 text-gray-200 mb-6" />
                        <h3 className="text-2xl font-bold mb-2">macOS</h3>
                        <p className="text-gray-400 text-sm mb-6">Universal Binary (Apple Silicon M1/M2 + Intel). Notarized .dmg package.</p>
                    </div>
                    <a href="https://github.com/brainRottedCoder/Ultraclaw/releases/latest" data-os="macos" target="_blank" rel="noreferrer" className="w-full py-4 rounded-xl bg-white/5 hover:bg-gray-200 hover:text-black transition-colors border border-white/10 flex items-center justify-center gap-2 font-bold group-hover:border-transparent z-10">
                        Download .dmg
                    </a>
                </motion.div>

                {/* LINUX */}
                <motion.div initial={{ opacity: 0, y: 20 }} animate={{ opacity: 1, y: 0 }} transition={{ delay: 0.5 }} className="glass-panel p-8 rounded-2xl hover:-translate-y-2 transition-transform duration-300 border border-white/5 group flex flex-col justify-between h-full relative overflow-hidden">
                    <div className="absolute inset-0 bg-orange-500/5 opacity-0 group-hover:opacity-100 transition-opacity"></div>
                    <div>
                        <Terminal className="w-10 h-10 text-orange-400 mb-6" />
                        <h3 className="text-2xl font-bold mb-2">Linux</h3>
                        <p className="text-gray-400 text-sm mb-6">AppImage packaged with Linux Landlock filesystem bounding security rules.</p>
                    </div>
                    <a href="https://github.com/brainRottedCoder/Ultraclaw/releases/latest" data-os="linux" target="_blank" rel="noreferrer" className="w-full py-4 rounded-xl bg-white/5 hover:bg-orange-500 hover:text-white transition-colors border border-white/10 flex items-center justify-center gap-2 font-bold group-hover:border-transparent z-10">
                        Get AppImage
                    </a>
                </motion.div>

                {/* ANDROID */}
                <motion.div initial={{ opacity: 0, y: 20 }} animate={{ opacity: 1, y: 0 }} transition={{ delay: 0.6 }} className="glass-panel p-8 rounded-2xl hover:-translate-y-2 transition-transform duration-300 border border-white/5 group flex flex-col justify-between h-full relative overflow-hidden">
                    <div className="absolute inset-0 bg-emerald-500/5 opacity-0 group-hover:opacity-100 transition-opacity"></div>
                    <div>
                        <Smartphone className="w-10 h-10 text-emerald-400 mb-6" />
                        <h3 className="text-2xl font-bold mb-2">Android</h3>
                        <p className="text-gray-400 text-sm mb-6">Sideloadable mobile interface built via Tauri Mobile compilation endpoints.</p>
                    </div>
                    <a href="https://github.com/brainRottedCoder/Ultraclaw/releases/latest" data-os="android" target="_blank" rel="noreferrer" className="w-full py-4 rounded-xl bg-white/5 hover:bg-emerald-500 hover:text-white transition-colors border border-white/10 flex items-center justify-center gap-2 font-bold group-hover:border-transparent z-10">
                        Download .apk
                    </a>
                </motion.div>

            </div>

            <motion.div initial={{ opacity: 0, y: 20 }} animate={{ opacity: 1, y: 0 }} transition={{ delay: 0.8 }} className="mt-20 glass-panel p-10 rounded-2xl border border-indigo-500/20 text-center max-w-4xl mx-auto flex flex-col md:flex-row items-center justify-between">
                <div className="text-left mb-6 md:mb-0">
                    <h2 className="text-2xl font-bold mb-2 flex items-center gap-3"><Package className="text-indigo-400" /> Developer Build</h2>
                    <p className="text-gray-400">Clone the Matrix from source and boot the Terminal Interface natively with Rust Cargo.</p>
                </div>
                <div className="bg-black/80 px-6 py-4 rounded-xl border border-white/10 font-mono text-sm text-indigo-300 flex items-center gap-4">
                    <span>git clone https://github.com/brainRottedCoder/Ultraclaw.git</span>
                </div>
            </motion.div>

        </div>
    );
}
