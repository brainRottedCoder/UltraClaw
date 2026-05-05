import React from 'react';
import { BrowserRouter as Router, Routes, Route, Link, useLocation } from 'react-router-dom';
import { HelmetProvider } from 'react-helmet-async';
import { Terminal, Download, Github, Star } from 'lucide-react';
import Home from './pages/Home';
import Downloads from './pages/Downloads';
import './index.css';

function Navbar() {
  const location = useLocation();

  return (
    <nav className="fixed top-0 w-full z-50 glass-panel border-b border-white/5 py-4 px-6 md:px-12 flex items-center justify-between">
      <Link to="/" className="flex items-center gap-3 group">
        <div className="w-8 h-8 rounded-lg bg-black overflow-hidden shadow-[0_0_15px_rgba(99,102,241,0.5)] group-hover:shadow-[0_0_25px_rgba(99,102,241,0.8)] transition-all">
          <img src={`${import.meta.env.BASE_URL}logo.png`} alt="Ultraclaw Logo" className="w-full h-full object-cover" />
        </div>
        <span className="text-xl font-bold tracking-tight text-white group-hover:text-cyan-400 transition-colors">Ultraclaw</span>
      </Link>

      <div className="flex items-center gap-6">
        <Link to="/" className={`text-sm font-medium transition-colors ${location.pathname === '/' ? 'text-cyan-400' : 'text-gray-400 hover:text-white'}`}>
          Platform
        </Link>
        <Link to="/downloads" className={`text-sm font-medium flex items-center gap-2 transition-colors ${location.pathname === '/downloads' ? 'text-cyan-400' : 'text-gray-400 hover:text-white'}`}>
          <Download className="w-4 h-4" /> Downloads
        </Link>
        <a href="https://github.com/brainRottedCoder/Ultraclaw" target="_blank" rel="noreferrer" className="flex items-center gap-2 px-4 py-2 bg-white/10 hover:bg-white/20 text-white rounded-full transition-all text-sm font-medium border border-white/5">
          <Github className="w-4 h-4" /> GitHub
        </a>
      </div>
    </nav>
  );
}

export default function App() {
  return (
    <HelmetProvider>
      {/* The basename string must match exactly your repo name on github pages */}
      <Router basename="/UltraClaw">
        <div className="min-h-screen bg-[#050505] text-white font-sans selection:bg-indigo-500/30">
          <Navbar />
          <main className="pt-24 min-h-screen">
            <Routes>
              <Route path="/" element={<Home />} />
              <Route path="/downloads" element={<Downloads />} />
            </Routes>
          </main>

          <footer className="border-t border-white/10 py-12 text-center mt-20 relative overflow-hidden">
            <div className="absolute top-0 left-1/2 -translate-x-1/2 w-[500px] h-[500px] bg-cyan-500/5 rounded-full blur-[100px] -z-10"></div>
            <p className="text-gray-500 text-sm">© 2026 Ultraclaw AI Framework. Open-Source under MIT.</p>
            <p className="text-gray-600 text-xs mt-2 flex items-center justify-center gap-1">Written with <Star className="w-3 h-3 text-yellow-500" /> globally.</p>
          </footer>
        </div>
      </Router>
    </HelmetProvider>
  );
}
