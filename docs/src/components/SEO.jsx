import React from 'react';
import { Helmet } from 'react-helmet-async';

export default function SEO({ title, description, path = "" }) {
    const url = `https://brainRottedCoder.github.io/Ultraclaw${path}`;
    const sitename = "Ultraclaw AI Framework";

    return (
        <Helmet>
            {/* Standard SEO */}
            <title>{title ? `${title} | ${sitename}` : sitename}</title>
            <meta name="description" content={description} />
            <meta name="keywords" content="autonomous agent, open source AI, AI framework, Rust LLM, openclaw, local AI, privacy AI, github pages" />

            {/* OpenGraph / Facebook */}
            <meta property="og:type" content="website" />
            <meta property="og:url" content={url} />
            <meta property="og:title" content={title || sitename} />
            <meta property="og:description" content={description} />
            <meta property="og:site_name" content={sitename} />

            {/* Twitter */}
            <meta name="twitter:card" content="summary_large_image" />
            <meta name="twitter:url" content={url} />
            <meta name="twitter:title" content={title || sitename} />
            <meta name="twitter:description" content={description} />

            {/* GEO Meta Tags */}
            <meta name="geo.region" content="US" />
            <meta name="geo.placename" content="Global" />

            {/* Canonical Link */}
            <link rel="canonical" href={url} />

            {/* JSON-LD Schema.org Data */}
            <script type="application/ld+json">
                {JSON.stringify({
                    "@context": "https://schema.org",
                    "@type": "SoftwareApplication",
                    "name": "Ultraclaw",
                    "description": description,
                    "applicationCategory": "DeveloperApplication",
                    "operatingSystem": "Windows, macOS, Linux, Android",
                    "offers": {
                        "@type": "Offer",
                        "price": "0",
                        "priceCurrency": "USD"
                    }
                })}
            </script>
        </Helmet>
    );
}
